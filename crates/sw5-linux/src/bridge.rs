//! Puente en Linux: volante real (evdev, sin force feedback en el kernel) →
//! volante virtual (uinput, con force feedback) → motores (hidraw).
//!
//! ```text
//! /dev/input/eventN (real, en grab exclusivo) ── ejes y botones ──▶ uinput "Speed Wheel 5 Pro"
//! juegos (SDL, Steam, Proton) ── FF upload/play ──▶ uinput ──▶ sw5_core::Engine ──▶ /dev/hidrawN
//! ```
//!
//! La regla de udev que instala `sw5-linux install` oculta el dispositivo real a
//! los usuarios, para que los juegos vean solo el virtual.

use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::os::fd::{AsRawFd, RawFd};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use evdev::uinput::VirtualDevice;
use evdev::{
    AttributeSet, Device, EventSummary, FFEffectCode, FFEffectData, FFEffectKind, FFWaveform, InputEvent, UInputCode,
    UinputAbsSetup,
};
use sw5_core::{ConditionKind, Engine, OutputConfig, PIDS, STOP_REPORT, VID, Waveform, encode_report};

use crate::ff::{self, LinuxEffect, LinuxEnvelope, LinuxKind};

/// Nombre del volante virtual: el mismo que el real, para que los juegos lo muestren igual.
pub const VIRTUAL_NAME: &str = "Speed Wheel 5 Pro";
const MAX_EFFECTS: u32 = 16;

#[derive(Debug, Clone)]
pub struct Options {
    pub output: OutputConfig,
    pub tick_ms: u64,
    pub keepalive_ms: u64,
    pub verbose: bool,
}

static STOP: AtomicBool = AtomicBool::new(false);

extern "C" fn on_signal(_: libc::c_int) {
    STOP.store(true, Ordering::SeqCst);
}

/// SIGTERM/SIGINT sin SA_RESTART: `poll` vuelve con EINTR y el bucle apaga los motores.
pub fn install_signal_handlers() {
    unsafe {
        let mut sa: libc::sigaction = std::mem::zeroed();
        sa.sa_sigaction = on_signal as *const () as usize;
        libc::sigemptyset(&mut sa.sa_mask);
        libc::sigaction(libc::SIGTERM, &sa, std::ptr::null_mut());
        libc::sigaction(libc::SIGINT, &sa, std::ptr::null_mut());
    }
}

pub fn stop_requested() -> bool {
    STOP.load(Ordering::SeqCst)
}

#[derive(Debug, Clone)]
pub struct Wheel {
    pub event: PathBuf,
    pub hidraw: PathBuf,
    pub name: String,
    pub pid: u16,
}

/// El nodo pertenece a un dispositivo virtual (uinput), como el nuestro.
fn is_virtual(event: &Path) -> bool {
    let Some(name) = event.file_name() else { return false };
    std::fs::canonicalize(Path::new("/sys/class/input").join(name))
        .map(|p| p.to_string_lossy().contains("/virtual/"))
        .unwrap_or(false)
}

/// `/dev/hidrawN` del volante, leyendo `HID_ID=0003:00001A5C:0000PPPP` en sysfs.
fn find_hidraw(pid: u16) -> Option<PathBuf> {
    let wanted = format!("HID_ID=0003:{:08X}:{:08X}", VID, pid);
    let mut found: Vec<_> = std::fs::read_dir("/sys/class/hidraw")
        .ok()?
        .flatten()
        .filter(|e| {
            std::fs::read_to_string(e.path().join("device/uevent"))
                .map(|u| u.lines().any(|l| l.eq_ignore_ascii_case(&wanted)))
                .unwrap_or(false)
        })
        .map(|e| Path::new("/dev").join(e.file_name()))
        .collect();
    found.sort();
    found.into_iter().next()
}

/// Volantes reales conectados (excluye el virtual).
pub fn find_wheels() -> Vec<Wheel> {
    let mut out: Vec<Wheel> = evdev::enumerate()
        .filter(|(path, dev)| {
            let id = dev.input_id();
            id.vendor() == VID && PIDS.contains(&id.product()) && !is_virtual(path)
        })
        .filter_map(|(path, dev)| {
            let pid = dev.input_id().product();
            Some(Wheel { event: path, hidraw: find_hidraw(pid)?, name: dev.name().unwrap_or("?").to_string(), pid })
        })
        .collect();
    out.sort_by(|a, b| a.event.cmp(&b.event));
    out
}

/// Nodos de los volantes virtuales creados por el servicio.
pub fn find_virtual() -> Vec<PathBuf> {
    evdev::enumerate()
        .filter(|(path, dev)| dev.input_id().vendor() == VID && is_virtual(path))
        .map(|(path, _)| path)
        .collect()
}

pub fn write_report(hid: &mut File, report: &[u8; 5]) -> io::Result<()> {
    // hidraw: el primer byte es el Report ID (0 = el dispositivo no usa IDs).
    hid.write_all(report)
}

fn set_nonblocking(fd: RawFd) -> io::Result<()> {
    unsafe {
        let flags = libc::fcntl(fd, libc::F_GETFL);
        if flags < 0 || libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) < 0 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}

/// Espera eventos en `fds` (como mucho `timeout_ms`). Devuelve qué fds están listos.
fn wait(fds: [RawFd; 2], timeout_ms: i32) -> io::Result<[bool; 2]> {
    let mut p = fds.map(|fd| libc::pollfd { fd, events: libc::POLLIN, revents: 0 });
    let r = unsafe { libc::poll(p.as_mut_ptr(), 2, timeout_ms) };
    if r < 0 {
        let e = io::Error::last_os_error();
        if e.kind() == io::ErrorKind::Interrupted {
            return Ok([false, false]);
        }
        return Err(e);
    }
    for x in &p {
        if x.revents & (libc::POLLERR | libc::POLLHUP | libc::POLLNVAL) != 0 {
            return Err(io::Error::new(io::ErrorKind::NotConnected, "el volante se desconectó"));
        }
    }
    Ok([p[0].revents & libc::POLLIN != 0, p[1].revents & libc::POLLIN != 0])
}

fn envelope(e: evdev::FFEnvelope) -> LinuxEnvelope {
    LinuxEnvelope {
        attack_length_ms: e.attack_length,
        attack_level: e.attack_level,
        fade_length_ms: e.fade_length,
        fade_level: e.fade_level,
    }
}

fn from_evdev(d: &FFEffectData) -> LinuxEffect {
    let kind = match d.kind {
        FFEffectKind::Rumble { strong_magnitude, weak_magnitude } => {
            LinuxKind::Rumble { strong: strong_magnitude, weak: weak_magnitude }
        }
        FFEffectKind::Constant { level, envelope: e } => LinuxKind::Constant { level, envelope: envelope(e) },
        FFEffectKind::Ramp { start_level, end_level, envelope: e } => {
            LinuxKind::Ramp { start: start_level, end: end_level, envelope: envelope(e) }
        }
        FFEffectKind::Periodic { waveform, period, magnitude, offset, phase, envelope: e } => LinuxKind::Periodic {
            waveform: match waveform {
                FFWaveform::Square => Waveform::Square,
                FFWaveform::Triangle => Waveform::Triangle,
                FFWaveform::Sine => Waveform::Sine,
                FFWaveform::SawUp => Waveform::SawtoothUp,
                FFWaveform::SawDown => Waveform::SawtoothDown,
            },
            period_ms: period,
            magnitude,
            offset,
            phase,
            envelope: envelope(e),
        },
        FFEffectKind::Spring { .. } => LinuxKind::Condition(ConditionKind::Spring),
        FFEffectKind::Friction { .. } => LinuxKind::Condition(ConditionKind::Friction),
        FFEffectKind::Damper => LinuxKind::Condition(ConditionKind::Damper),
        FFEffectKind::Inertia => LinuxKind::Condition(ConditionKind::Inertia),
    };
    LinuxEffect { kind, length_ms: d.replay.length, delay_ms: d.replay.delay }
}

fn build_virtual(src: &Device) -> Result<VirtualDevice> {
    let mut ff = AttributeSet::<FFEffectCode>::new();
    for c in [
        FFEffectCode::FF_RUMBLE,
        FFEffectCode::FF_PERIODIC,
        FFEffectCode::FF_CONSTANT,
        FFEffectCode::FF_RAMP,
        FFEffectCode::FF_SPRING,
        FFEffectCode::FF_FRICTION,
        FFEffectCode::FF_DAMPER,
        FFEffectCode::FF_INERTIA,
        FFEffectCode::FF_SQUARE,
        FFEffectCode::FF_TRIANGLE,
        FFEffectCode::FF_SINE,
        FFEffectCode::FF_SAW_UP,
        FFEffectCode::FF_SAW_DOWN,
        FFEffectCode::FF_GAIN,
    ] {
        ff.insert(c);
    }
    let mut b = VirtualDevice::builder()?.name(VIRTUAL_NAME).input_id(src.input_id());
    if let Some(keys) = src.supported_keys() {
        b = b.with_keys(keys)?;
    }
    for (code, info) in src.get_absinfo()? {
        b = b.with_absolute_axis(&UinputAbsSetup::new(code, info))?;
    }
    Ok(b.with_ff(&ff)?.with_ff_effects_max(MAX_EFFECTS).build()?)
}

/// Atiende un volante hasta que se desconecta o llega SIGTERM.
pub fn run_wheel(w: &Wheel, opts: &Options) -> Result<()> {
    let mut src = Device::open(&w.event).with_context(|| format!("no se pudo abrir {}", w.event.display()))?;
    let mut hid = OpenOptions::new()
        .write(true)
        .open(&w.hidraw)
        .with_context(|| format!("no se pudo abrir {}", w.hidraw.display()))?;
    let mut vdev =
        build_virtual(&src).context("no se pudo crear el volante virtual (¿está cargado el módulo uinput?)")?;
    src.grab().context("no se pudo tomar el volante en modo exclusivo")?;
    src.set_nonblocking(true)?;
    set_nonblocking(vdev.as_raw_fd())?;
    eprintln!("sw5-linux: atendiendo {} ({} + {})", w.name, w.event.display(), w.hidraw.display());

    let epoch = Instant::now();
    let now_us = || epoch.elapsed().as_micros() as u64;
    let mut engine = Engine::new(MAX_EFFECTS as usize);
    let mut handles: HashMap<i16, u32> = HashMap::new();
    let mut last = STOP_REPORT;
    let mut last_sent = Instant::now();
    let mut idle = true;
    let keepalive = Duration::from_millis(opts.keepalive_ms);
    let result = loop {
        if stop_requested() {
            break Ok(());
        }
        let timeout = if idle { 1000 } else { opts.tick_ms as i32 };
        let ready = match wait([src.as_raw_fd(), vdev.as_raw_fd()], timeout) {
            Ok(r) => r,
            Err(e) => break Err(e.into()),
        };

        // Volante real → virtual: ejes y botones tal cual.
        if ready[0] {
            let batch: Vec<InputEvent> = match src.fetch_events() {
                Ok(evs) => evs.filter(|e| !matches!(e.destructure(), EventSummary::Synchronization(..))).collect(),
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => Vec::new(),
                Err(e) => break Err(anyhow::Error::from(e).context("el volante se desconectó")),
            };
            if !batch.is_empty() {
                if let Err(e) = vdev.emit(&batch) {
                    break Err(e.into());
                }
            }
        }

        // Juegos → efectos.
        if ready[1] {
            let events: Vec<InputEvent> = match vdev.fetch_events() {
                Ok(evs) => evs.collect(),
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => Vec::new(),
                Err(e) => break Err(e.into()),
            };
            for ev in events {
                match ev.destructure() {
                    EventSummary::UInput(ev, UInputCode::UI_FF_UPLOAD, ..) => {
                        let mut up = match vdev.process_ff_upload(ev) {
                            Ok(u) => u,
                            Err(e) => {
                                eprintln!("sw5-linux: error en FF upload: {e}");
                                continue;
                            }
                        };
                        let id = up.effect_id();
                        // evdev hace unreachable!() con formas de onda desconocidas: que no tumbe el servicio.
                        let data = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| up.effect()));
                        let Ok(data) = data else {
                            up.set_retval(-libc::EINVAL);
                            continue;
                        };
                        let effect = ff::to_core(&from_evdev(&data));
                        if opts.verbose {
                            eprintln!("sw5-linux: upload id={id} {effect:?}");
                        }
                        let r = match handles.get(&id) {
                            Some(&h) => engine.update(h, effect).map(|_| h),
                            None => engine.create(effect),
                        };
                        match r {
                            Ok(h) => {
                                handles.insert(id, h);
                                up.set_retval(0);
                            }
                            Err(_) => up.set_retval(-libc::ENOSPC),
                        }
                    }
                    EventSummary::UInput(ev, UInputCode::UI_FF_ERASE, ..) => {
                        if let Ok(er) = vdev.process_ff_erase(ev) {
                            if let Some(h) = handles.remove(&(er.effect_id() as i16)) {
                                let _ = engine.destroy(h);
                            }
                        }
                    }
                    EventSummary::ForceFeedback(_, FFEffectCode::FF_GAIN, value) => engine.set_gain(ff::gain(value)),
                    EventSummary::ForceFeedback(_, FFEffectCode::FF_AUTOCENTER, _) => {}
                    EventSummary::ForceFeedback(_, code, value) => {
                        if let Some(&h) = handles.get(&(code.0 as i16)) {
                            if opts.verbose {
                                eprintln!("sw5-linux: play id={} count={value}", code.0);
                            }
                            let _ =
                                if value > 0 { engine.start(h, value as u32, false, now_us()) } else { engine.stop(h) };
                        }
                    }
                    _ => {}
                }
            }
        }

        // Mezcla → motores.
        let now = now_us();
        let levels = engine.mix(now);
        let stopped = engine.state(now).stopped;
        let report = encode_report(levels, &opts.output);
        let keepalive_due = report != STOP_REPORT && !keepalive.is_zero() && last_sent.elapsed() >= keepalive;
        if report != last || keepalive_due {
            if let Err(e) = write_report(&mut hid, &report) {
                break Err(anyhow::Error::from(e).context("no se pudo escribir al volante"));
            }
            last = report;
            last_sent = Instant::now();
        }
        idle = stopped && report == STOP_REPORT;
    };
    let _ = write_report(&mut hid, &STOP_REPORT);
    result
}

/// Bucle del servicio: espera el volante, lo atiende y vuelve a esperar si se desconecta.
pub fn run_forever(opts: &Options) -> Result<()> {
    install_signal_handlers();
    let mut announced = false;
    while !stop_requested() {
        match find_wheels().into_iter().next() {
            Some(w) => {
                announced = false;
                if let Err(e) = run_wheel(&w, opts) {
                    eprintln!("sw5-linux: {e:#}");
                }
            }
            None if !announced => {
                eprintln!("sw5-linux: esperando un Speed Wheel 5 Pro (VID {VID:04x})…");
                announced = true;
            }
            None => {}
        }
        for _ in 0..20 {
            if stop_requested() {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
    eprintln!("sw5-linux: detenido");
    Ok(())
}

/// Report crudo durante `ms` (para `sw5-linux raw`). Falla si el servicio tiene el volante.
pub fn raw(a: u8, b: u8, ms: u64) -> Result<()> {
    let Some(w) = find_wheels().into_iter().next() else { bail!("no hay ningún Speed Wheel 5 Pro conectado") };
    let mut hid = OpenOptions::new()
        .write(true)
        .open(&w.hidraw)
        .with_context(|| format!("no se pudo abrir {} (¿falta sudo?)", w.hidraw.display()))?;
    let report = [0, 0, 0, a, b];
    println!("enviando {report:02X?} durante {ms} ms a {}", w.hidraw.display());
    let end = Instant::now() + Duration::from_millis(ms);
    while Instant::now() < end {
        write_report(&mut hid, &report)?;
        std::thread::sleep(Duration::from_millis(50));
    }
    write_report(&mut hid, &STOP_REPORT)?;
    Ok(())
}

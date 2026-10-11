//! Un volante abierto: el motor de efectos compartido y el hilo que lo mezcla y
//! envía los reports.
//!
//! El hilo se crea con `CreateThread` (no `std::thread`) porque tiene que salir con
//! `FreeLibraryAndExitThread`: DirectInput hace `FreeLibrary` de la DLL por su
//! cuenta cuando suelta el último objeto, y un hilo vivo ejecutando código de una
//! DLL descargada tumba el juego. El hilo fija la DLL con `GetModuleHandleExW` y la
//! suelta al salir, después de haber liberado todo lo suyo.

use core::ffi::c_void;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use sw5_core::{Engine, STOP_REPORT, encode_report};
use sw5_win::hid::HidDevice;
use windows::Win32::Foundation::{CloseHandle, FreeLibrary, HANDLE, HMODULE, WAIT_OBJECT_0};
use windows::Win32::System::LibraryLoader::{
    FreeLibraryAndExitThread, GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS, GetModuleHandleExW,
};
use windows::Win32::System::Threading::{
    CREATE_WAITABLE_TIMER_HIGH_RESOLUTION, CreateEventW, CreateThread, CreateWaitableTimerExW, INFINITE, SetEvent,
    SetWaitableTimer, THREAD_CREATION_FLAGS, TIMER_ALL_ACCESS, WaitForMultipleObjects, WaitForSingleObject,
};
use windows_core::{HRESULT, PCWSTR};

use crate::config::Config;
use crate::log::log;

/// Máximo de efectos descargados por dispositivo.
const MAX_EFFECTS: usize = 64;

struct OwnedHandle(HANDLE);
unsafe impl Send for OwnedHandle {}
unsafe impl Sync for OwnedHandle {}
impl Drop for OwnedHandle {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

/// Estado compartido entre los métodos COM (hilo del juego) y el hilo del dispositivo.
pub struct Shared {
    engine: Mutex<Engine>,
    stop: OwnedHandle,
    wake: OwnedHandle,
    epoch: Instant,
    cfg: Config,
}

impl Shared {
    pub fn now_us(&self) -> u64 {
        self.epoch.elapsed().as_micros() as u64
    }

    pub fn engine(&self) -> MutexGuard<'_, Engine> {
        self.engine.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Avisa al hilo que algo cambió, para que no espere al próximo tick estando en reposo.
    pub fn wake(&self) {
        unsafe {
            let _ = SetEvent(self.wake.0);
        }
    }
}

pub struct Device {
    pub path: Vec<u16>,
    pub version: u16,
    shared: Arc<Shared>,
    thread: OwnedHandle,
}

struct WorkerCtx {
    shared: Arc<Shared>,
    hid: HidDevice,
    path: Vec<u16>,
    module: HMODULE,
}

impl Device {
    pub fn open(path: Vec<u16>, cfg: &Config) -> Result<Device, HRESULT> {
        let hid = HidDevice::open(&path).map_err(|e| e.code())?;
        if (hid.caps.output_len as usize) < sw5_core::REPORT_LEN {
            log!("output report de {} bytes: no es un Speed Wheel 5 Pro", hid.caps.output_len);
            return Err(windows::Win32::Foundation::E_FAIL);
        }
        let version = sw5_win::hid::query(&path).map(|i| i.version).unwrap_or(0);
        let event = || unsafe { CreateEventW(None, false, false, PCWSTR::null()) }.map(OwnedHandle);
        let shared = Arc::new(Shared {
            engine: Mutex::new(Engine::new(MAX_EFFECTS)),
            stop: event().map_err(|e| e.code())?,
            wake: event().map_err(|e| e.code())?,
            epoch: Instant::now(),
            cfg: cfg.clone(),
        });

        let mut module = HMODULE::default();
        unsafe {
            GetModuleHandleExW(GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS, PCWSTR(worker_main as *const u16), &mut module)
        }
        .map_err(|e| e.code())?;
        let ctx = Box::into_raw(Box::new(WorkerCtx { shared: shared.clone(), hid, path: path.clone(), module }));
        let thread = match unsafe {
            CreateThread(None, 0, Some(worker_main), Some(ctx as *const c_void), THREAD_CREATION_FLAGS(0), None)
        } {
            Ok(h) => OwnedHandle(h),
            Err(e) => {
                unsafe {
                    drop(Box::from_raw(ctx));
                    let _ = FreeLibrary(module);
                }
                return Err(e.code());
            }
        };
        Ok(Device { path, version, shared, thread })
    }

    pub fn shared(&self) -> Arc<Shared> {
        self.shared.clone()
    }
}

impl Drop for Device {
    fn drop(&mut self) {
        unsafe {
            let _ = SetEvent(self.shared.stop.0);
            if WaitForSingleObject(self.thread.0, 3000) != WAIT_OBJECT_0 {
                log!("el hilo del dispositivo no terminó en 3 s");
            }
        }
    }
}

unsafe extern "system" fn worker_main(param: *mut c_void) -> u32 {
    let ctx = unsafe { Box::from_raw(param as *mut WorkerCtx) };
    let module = ctx.module;
    if std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || worker_loop(*ctx))).is_err() {
        log!("panic en el hilo del dispositivo");
    }
    // Todo lo del hilo ya se liberó dentro de worker_loop.
    unsafe { FreeLibraryAndExitThread(module, 0) }
}

fn create_timer(tick_ms: u32) -> Option<OwnedHandle> {
    let make = |flags| unsafe { CreateWaitableTimerExW(None, PCWSTR::null(), flags, TIMER_ALL_ACCESS.0) };
    // Los timers de alta resolución existen desde Windows 10 1803.
    let h = make(CREATE_WAITABLE_TIMER_HIGH_RESOLUTION).or_else(|_| make(0)).ok()?;
    let due = -(tick_ms as i64) * 10_000; // relativo, en unidades de 100 ns
    unsafe { SetWaitableTimer(h, &due, tick_ms as i32, None, None, false) }.ok()?;
    Some(OwnedHandle(h))
}

fn worker_loop(ctx: WorkerCtx) {
    let WorkerCtx { shared, mut hid, path, .. } = ctx;
    let cfg = &shared.cfg;
    let Some(timer) = create_timer(cfg.tick_ms) else {
        log!("no se pudo crear el timer");
        return;
    };
    let keepalive = Duration::from_millis(cfg.keepalive_ms as u64);
    log!("hilo del dispositivo iniciado (tick {} ms)", cfg.tick_ms);

    let mut last = STOP_REPORT;
    let mut last_sent = Instant::now();
    let mut idle = true;
    let mut failing = false;
    let mut last_reopen = Instant::now();
    loop {
        // En reposo no se despierta con el timer: solo con stop o wake.
        let all = [shared.stop.0, shared.wake.0, timer.0];
        let handles = if idle { &all[..2] } else { &all[..] };
        let r = unsafe { WaitForMultipleObjects(handles, false, if idle { INFINITE } else { 1000 }) };
        if r == WAIT_OBJECT_0 {
            break;
        }

        let now = shared.now_us();
        let (levels, stopped) = {
            let mut e = shared.engine();
            (e.mix(now), e.state(now).stopped)
        };
        let report = encode_report(levels, &cfg.output);
        let keepalive_due = report != STOP_REPORT && keepalive > Duration::ZERO && last_sent.elapsed() >= keepalive;
        if report != last || keepalive_due {
            match hid.write_report(&report, 100) {
                Ok(()) => {
                    if failing {
                        log!("escritura al volante recuperada");
                        failing = false;
                    }
                    last = report;
                    last_sent = Instant::now();
                }
                Err(e) => {
                    if !failing {
                        log!("error escribiendo al volante: {e}");
                        failing = true;
                    }
                    // Volante desconectado y vuelto a conectar: reabrir, como mucho 1 vez/s.
                    if last_reopen.elapsed() >= Duration::from_secs(1) {
                        last_reopen = Instant::now();
                        if let Ok(h) = HidDevice::open(&path) {
                            hid = h;
                        }
                    }
                }
            }
        }
        idle = stopped && report == STOP_REPORT && last == STOP_REPORT;
    }
    let _ = hid.write_report(&STOP_REPORT, 100);
    log!("hilo del dispositivo terminado");
}

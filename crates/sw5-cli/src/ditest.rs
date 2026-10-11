//! Prueba de punta a punta por DirectInput 8, como lo haría un juego: DirectInput
//! carga el effect driver registrado para el volante y le pasa los efectos.

use core::ffi::c_void;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use windows::Win32::Devices::HumanInterfaceDevice::{
    DI8DEVCLASS_GAMECTRL, DICONSTANTFORCE, DIDATAFORMAT, DIDEVICEINSTANCEW, DIDF_ABSAXIS, DIDFT_ANYINSTANCE,
    DIDFT_AXIS, DIDFT_BUTTON, DIEDFL_ATTACHEDONLY, DIEDFL_FORCEFEEDBACK, DIEFF_CARTESIAN, DIEFF_OBJECTOFFSETS,
    DIEFFECT, DIEGES_PLAYING, DIENUM_CONTINUE, DIOBJECTDATAFORMAT, DIPERIODIC, DIRAMPFORCE, DIRECTINPUT_VERSION,
    DISCL_BACKGROUND, DISCL_EXCLUSIVE, DirectInput8Create, GUID_ConstantForce, GUID_RampForce, GUID_Sine, GUID_Square,
    GUID_XAxis, GUID_YAxis, IDirectInput8W, IDirectInputDevice8W, IDirectInputEffect,
};
use windows::Win32::Foundation::{HINSTANCE, HWND};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{CreateWindowExW, DestroyWindow, WINDOW_EX_STYLE, WS_OVERLAPPED};
use windows_core::{BOOL, GUID, Interface, w};

use crate::out::say;

const DIDFT_OPTIONAL: u32 = 0x8000_0000;
const DIEB_NOTRIGGER: u32 = 0xFFFF_FFFF;
const OFS_X: u32 = 0;
const OFS_Y: u32 = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum TestEffect {
    /// Fuerza constante en 1 eje (lo típico de un juego de autos): los dos motores.
    Constant,
    /// Constante en 2 ejes con dirección +X: solo el motor fuerte.
    Strong,
    /// Constante en 2 ejes con dirección +Y: solo el motor suave.
    Weak,
    /// Seno de 250 ms: pulsos.
    Sine,
    /// Cuadrada de 500 ms: encendido/apagado.
    Square,
    /// Rampa de -100 % a +100 %: baja y vuelve a subir.
    Ramp,
    /// Dos efectos a la vez (constante 40 % + seno 40 %): se suman.
    Mix,
    /// Todos los anteriores, uno tras otro.
    All,
}

struct Window(HWND);
impl Drop for Window {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyWindow(self.0);
        }
    }
}

unsafe extern "system" fn enum_cb(inst: *mut DIDEVICEINSTANCEW, ctx: *mut c_void) -> BOOL {
    unsafe {
        let found = &mut *(ctx as *mut Vec<DIDEVICEINSTANCEW>);
        found.push(*inst);
    }
    BOOL(DIENUM_CONTINUE as i32)
}

/// Parámetros que tienen que vivir mientras DirectInput lee el DIEFFECT.
struct Params {
    axes: [u32; 2],
    dir: [i32; 2],
    constant: DICONSTANTFORCE,
    periodic: DIPERIODIC,
    ramp: DIRAMPFORCE,
}

fn create(
    dev: &IDirectInputDevice8W,
    guid: &GUID,
    n_axes: u32,
    ts: (*mut c_void, u32),
    ms: u64,
    p: &mut Params,
) -> Result<IDirectInputEffect> {
    let mut eff = DIEFFECT {
        dwSize: size_of::<DIEFFECT>() as u32,
        dwFlags: DIEFF_CARTESIAN | DIEFF_OBJECTOFFSETS,
        dwDuration: (ms * 1000) as u32,
        dwGain: 10_000,
        dwTriggerButton: DIEB_NOTRIGGER,
        cAxes: n_axes,
        rgdwAxes: p.axes.as_mut_ptr(),
        rglDirection: p.dir.as_mut_ptr(),
        cbTypeSpecificParams: ts.1,
        lpvTypeSpecificParams: ts.0,
        ..Default::default()
    };
    let mut out: Option<IDirectInputEffect> = None;
    unsafe { dev.CreateEffect(guid, &mut eff, &mut out, None) }.context("CreateEffect")?;
    out.context("CreateEffect no devolvió efecto")
}

fn play(name: &str, effects: &[IDirectInputEffect], ms: u64) -> Result<()> {
    say!("▶ {name} ({ms} ms)");
    for e in effects {
        unsafe { e.Start(1, 0) }.context("Start")?;
    }
    std::thread::sleep(Duration::from_millis(ms / 2));
    let mut status = 0u32;
    unsafe { effects[0].GetEffectStatus(&mut status) }.context("GetEffectStatus")?;
    say!(
        "  a mitad de camino el driver dice: {}",
        if status & DIEGES_PLAYING != 0 { "reproduciendo" } else { "detenido" }
    );
    let end = Instant::now() + Duration::from_millis(ms / 2 + 300);
    while Instant::now() < end {
        std::thread::sleep(Duration::from_millis(50));
    }
    unsafe { effects[0].GetEffectStatus(&mut status) }.context("GetEffectStatus")?;
    say!(
        "  al terminar la duración: {}",
        if status & DIEGES_PLAYING != 0 { "reproduciendo (¡mal!)" } else { "detenido" }
    );
    for e in effects {
        unsafe {
            let _ = e.Stop();
            let _ = e.Unload();
        }
    }
    std::thread::sleep(Duration::from_millis(500));
    Ok(())
}

pub fn run(which: TestEffect, ms: u64, magnitude: i32) -> Result<()> {
    unsafe {
        let hinst: HINSTANCE = GetModuleHandleW(None)?.into();
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("STATIC"),
            w!("sw5 ffb-test"),
            WS_OVERLAPPED,
            0,
            0,
            0,
            0,
            None,
            None,
            Some(hinst),
            None,
        )?;
        let _window = Window(hwnd);

        let mut di: Option<IDirectInput8W> = None;
        DirectInput8Create(hinst, DIRECTINPUT_VERSION, &IDirectInput8W::IID, (&raw mut di).cast(), None)
            .context("DirectInput8Create")?;
        let di = di.context("DirectInput8Create no devolvió interfaz")?;

        let mut found: Vec<DIDEVICEINSTANCEW> = Vec::new();
        di.EnumDevices(
            DI8DEVCLASS_GAMECTRL,
            Some(enum_cb),
            (&raw mut found).cast(),
            DIEDFL_ATTACHEDONLY | DIEDFL_FORCEFEEDBACK,
        )?;
        // guidProduct.data1 = PID << 16 | VID
        let Some(inst) = found.iter().find(|d| d.guidProduct.data1 & 0xFFFF == sw5_win::VID as u32) else {
            bail!(
                "DirectInput no ve ningún volante con force feedback (¿está instalado el driver? probá `sw5 status`)"
            );
        };
        say!(
            "DirectInput: \"{}\" — effect driver {{{:?}}}{}",
            sw5_win::from_wide(&inst.tszProductName),
            inst.guidFFDriver,
            if format!("{{{:?}}}", inst.guidFFDriver) == sw5_win::CLSID_STR { " (driver Rust)" } else { "" }
        );

        let mut dev: Option<IDirectInputDevice8W> = None;
        di.CreateDevice(&inst.guidInstance, &mut dev, None).context("CreateDevice")?;
        let dev = dev.context("CreateDevice no devolvió dispositivo")?;

        let mut objs = [
            DIOBJECTDATAFORMAT { pguid: &GUID_XAxis, dwOfs: OFS_X, dwType: DIDFT_AXIS | DIDFT_ANYINSTANCE, dwFlags: 0 },
            DIOBJECTDATAFORMAT {
                pguid: &GUID_YAxis,
                dwOfs: OFS_Y,
                dwType: DIDFT_AXIS | DIDFT_ANYINSTANCE | DIDFT_OPTIONAL,
                dwFlags: 0,
            },
            DIOBJECTDATAFORMAT {
                pguid: core::ptr::null(),
                dwOfs: 8,
                dwType: DIDFT_BUTTON | DIDFT_ANYINSTANCE | DIDFT_OPTIONAL,
                dwFlags: 0,
            },
        ];
        let mut fmt = DIDATAFORMAT {
            dwSize: size_of::<DIDATAFORMAT>() as u32,
            dwObjSize: size_of::<DIOBJECTDATAFORMAT>() as u32,
            dwFlags: DIDF_ABSAXIS,
            dwDataSize: 12,
            dwNumObjs: objs.len() as u32,
            rgodf: objs.as_mut_ptr(),
        };
        dev.SetDataFormat(&mut fmt).context("SetDataFormat")?;
        dev.SetCooperativeLevel(hwnd, DISCL_EXCLUSIVE | DISCL_BACKGROUND).context("SetCooperativeLevel")?;
        dev.Acquire().context("Acquire (¿otro programa tiene el volante en modo exclusivo?)")?;

        let mag = magnitude.clamp(0, 10_000);
        let mut p = Params {
            axes: [OFS_X, OFS_Y],
            dir: [1, 0],
            constant: DICONSTANTFORCE { lMagnitude: mag },
            periodic: DIPERIODIC { dwMagnitude: mag as u32, lOffset: 0, dwPhase: 0, dwPeriod: 250_000 },
            ramp: DIRAMPFORCE { lStart: -mag, lEnd: mag },
        };
        let cf = |p: &mut Params| ((&raw mut p.constant).cast(), size_of::<DICONSTANTFORCE>() as u32);
        let pf = |p: &mut Params| ((&raw mut p.periodic).cast(), size_of::<DIPERIODIC>() as u32);
        let rf = |p: &mut Params| ((&raw mut p.ramp).cast(), size_of::<DIRAMPFORCE>() as u32);

        let list: &[TestEffect] = if which == TestEffect::All {
            &[
                TestEffect::Constant,
                TestEffect::Strong,
                TestEffect::Weak,
                TestEffect::Sine,
                TestEffect::Square,
                TestEffect::Ramp,
                TestEffect::Mix,
            ]
        } else {
            std::slice::from_ref(&which)
        };
        for &t in list {
            match t {
                TestEffect::Constant => {
                    p.dir = [1, 0];
                    let ts = cf(&mut p);
                    play(
                        "constante, 1 eje → los dos motores",
                        &[create(&dev, &GUID_ConstantForce, 1, ts, ms, &mut p)?],
                        ms,
                    )?;
                }
                TestEffect::Strong => {
                    p.dir = [10_000, 0];
                    let ts = cf(&mut p);
                    play(
                        "constante, dirección +X → motor fuerte",
                        &[create(&dev, &GUID_ConstantForce, 2, ts, ms, &mut p)?],
                        ms,
                    )?;
                }
                TestEffect::Weak => {
                    p.dir = [0, 10_000];
                    let ts = cf(&mut p);
                    play(
                        "constante, dirección +Y → motor suave",
                        &[create(&dev, &GUID_ConstantForce, 2, ts, ms, &mut p)?],
                        ms,
                    )?;
                }
                TestEffect::Sine => {
                    p.dir = [1, 0];
                    p.periodic.dwPeriod = 250_000;
                    let ts = pf(&mut p);
                    play("seno de 250 ms → pulsos", &[create(&dev, &GUID_Sine, 1, ts, ms, &mut p)?], ms)?;
                }
                TestEffect::Square => {
                    p.dir = [1, 0];
                    p.periodic.dwPeriod = 500_000;
                    let ts = pf(&mut p);
                    play(
                        "cuadrada de 500 ms → encendido/apagado",
                        &[create(&dev, &GUID_Square, 1, ts, ms, &mut p)?],
                        ms,
                    )?;
                }
                TestEffect::Ramp => {
                    p.dir = [1, 0];
                    let ts = rf(&mut p);
                    play(
                        "rampa -100 % → +100 % → baja y sube",
                        &[create(&dev, &GUID_RampForce, 1, ts, ms, &mut p)?],
                        ms,
                    )?;
                }
                TestEffect::Mix => {
                    p.dir = [1, 0];
                    p.constant.lMagnitude = mag * 4 / 10;
                    p.periodic.dwMagnitude = (mag * 4 / 10) as u32;
                    p.periodic.dwPeriod = 250_000;
                    let ts = cf(&mut p);
                    let a = create(&dev, &GUID_ConstantForce, 1, ts, ms, &mut p)?;
                    let ts = pf(&mut p);
                    let b = create(&dev, &GUID_Sine, 1, ts, ms, &mut p)?;
                    play("constante 40 % + seno 40 % a la vez → se suman", &[a, b], ms)?;
                    p.constant.lMagnitude = mag;
                    p.periodic.dwMagnitude = mag as u32;
                }
                TestEffect::All => unreachable!(),
            }
        }
        let _ = dev.Unacquire();
    }
    Ok(())
}

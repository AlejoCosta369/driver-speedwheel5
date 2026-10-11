//! Autoprueba sin instalar: carga la DLL con `LoadLibrary`, crea el driver por COM
//! y le habla directo, igual que DirectInput, pero sin tocar el registro.
//! Al final descarga la DLL para comprobar que el hilo del dispositivo la suelta bien.

use core::ffi::c_void;
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use sw5_effectdrv::{CLSID, IDirectInputEffectDriver};
use sw5_win::oem::{ID_CONSTANT, ID_SINE};
use windows::Win32::Devices::HumanInterfaceDevice::{
    DICONSTANTFORCE, DIDEVICESTATE, DIEFF_CARTESIAN, DIEFFECT, DIEGES_PLAYING, DIEP_START, DIHIDFFINITINFO, DIPERIODIC,
};
use windows::Win32::Foundation::FreeLibrary;
use windows::Win32::System::Com::IClassFactory;
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows_core::{GUID, HRESULT, HSTRING, Interface, PWSTR, s};

use crate::out::say;

type GetClassObject = unsafe extern "system" fn(*const GUID, *const GUID, *mut *mut c_void) -> HRESULT;
type CanUnloadNow = unsafe extern "system" fn() -> HRESULT;

fn effect(ts: *mut c_void, cb: u32, ms: u32, axes: &mut [u32; 1], dir: &mut [i32; 1]) -> DIEFFECT {
    DIEFFECT {
        dwSize: size_of::<DIEFFECT>() as u32,
        dwFlags: DIEFF_CARTESIAN,
        dwDuration: ms * 1000,
        dwGain: 10_000,
        dwTriggerButton: u32::MAX,
        cAxes: 1,
        rgdwAxes: axes.as_mut_ptr(),
        rglDirection: dir.as_mut_ptr(),
        cbTypeSpecificParams: cb,
        lpvTypeSpecificParams: ts,
        ..Default::default()
    }
}

pub fn run(dll: &Path) -> Result<()> {
    let wheel = sw5_win::hid::find_wheels()?.into_iter().next().context("no hay volante conectado")?;
    say!("cargando {}", dll.display());
    unsafe {
        let module = LoadLibraryW(&HSTRING::from(dll.as_os_str())).context("LoadLibraryW")?;
        let get: GetClassObject =
            std::mem::transmute(GetProcAddress(module, s!("DllGetClassObject")).context("falta DllGetClassObject")?);
        let can_unload: CanUnloadNow =
            std::mem::transmute(GetProcAddress(module, s!("DllCanUnloadNow")).context("falta DllCanUnloadNow")?);

        let mut raw = core::ptr::null_mut();
        get(&CLSID, &IClassFactory::IID, &mut raw).ok().context("DllGetClassObject")?;
        let factory = IClassFactory::from_raw(raw);
        let drv: IDirectInputEffectDriver = factory.CreateInstance(None).context("CreateInstance")?;
        drop(factory);
        say!("driver creado; DllCanUnloadNow = {:?} (esperado S_FALSE)", can_unload());

        let mut path = wheel.path.clone();
        let mut info = DIHIDFFINITINFO {
            dwSize: size_of::<DIHIDFFINITINFO>() as u32,
            pwszDeviceInterface: PWSTR(path.as_mut_ptr()),
            GuidInstance: GUID::zeroed(),
        };
        drv.DeviceID(0x0800, 0, 1, 0, (&raw mut info).cast()).ok().context("DeviceID(begin)")?;
        say!("DeviceID(begin) ok");

        let (mut axes, mut dir) = ([0u32], [1i32]);
        let mut c = DICONSTANTFORCE { lMagnitude: 8000 };
        let e = effect((&raw mut c).cast(), size_of::<DICONSTANTFORCE>() as u32, 1200, &mut axes, &mut dir);
        let mut h1 = 0u32;
        drv.DownloadEffect(0, ID_CONSTANT, &mut h1, &e, DIEP_START).ok().context("DownloadEffect(constante)")?;
        say!("▶ constante 80 % durante 1,2 s (handle {h1})");
        std::thread::sleep(Duration::from_millis(600));
        let mut status = 0u32;
        drv.GetEffectStatus(0, h1, &mut status).ok()?;
        if status & DIEGES_PLAYING == 0 {
            bail!("GetEffectStatus dice detenido a mitad del efecto");
        }
        std::thread::sleep(Duration::from_millis(1000));
        drv.GetEffectStatus(0, h1, &mut status).ok()?;
        if status & DIEGES_PLAYING != 0 {
            bail!("GetEffectStatus dice reproduciendo después de la duración");
        }
        say!("  estado correcto antes y después de la duración");

        let mut p = DIPERIODIC { dwMagnitude: 10_000, lOffset: 0, dwPhase: 0, dwPeriod: 300_000 };
        let e = effect((&raw mut p).cast(), size_of::<DIPERIODIC>() as u32, 1500, &mut axes, &mut dir);
        let mut h2 = 0u32;
        drv.DownloadEffect(0, ID_SINE, &mut h2, &e, 0).ok().context("DownloadEffect(seno)")?;
        drv.StartEffect(0, h2, 0, 1).ok().context("StartEffect")?;
        say!("▶ seno de 300 ms durante 1,5 s (handle {h2})");
        std::thread::sleep(Duration::from_millis(1700));

        let mut st = DIDEVICESTATE { dwSize: size_of::<DIDEVICESTATE>() as u32, ..Default::default() };
        drv.GetForceFeedbackState(0, &mut st).ok()?;
        say!("GetForceFeedbackState: estado {:#x}, carga {} %", st.dwState, st.dwLoad);
        if drv.DestroyEffect(0, 999).is_ok() {
            bail!("DestroyEffect con handle inválido debería fallar");
        }
        drv.DestroyEffect(0, h1).ok()?;
        drv.DestroyEffect(0, h2).ok()?;

        drv.DeviceID(0x0800, 0, 0, 0, core::ptr::null_mut()).ok().context("DeviceID(end)")?;
        drop(drv);
        say!("driver liberado; DllCanUnloadNow = {:?} (esperado S_OK)", can_unload());
        FreeLibrary(module)?;
        std::thread::sleep(Duration::from_millis(300));
        say!("DLL descargada sin problemas: autoprueba OK");
    }
    Ok(())
}

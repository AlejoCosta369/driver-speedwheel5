//! Effect driver de DirectInput para el Speed Wheel 5 Pro: servidor COM in-proc.
//!
//! DirectInput lo carga dentro del proceso del juego porque la clave OEM del
//! volante apunta a [`CLSID`] (ver `sw5_win::oem`). Reglas de esta DLL:
//!
//! * Nada de panics cruzando la frontera FFI: todo método y export pasa por [`guard`].
//! * Sin `DllMain`: no hay nada que hacer al cargar, y así no hay riesgos con el
//!   loader lock.
//! * Nada de E/S en el camino crítico salvo la escritura al volante (en su hilo).

#![cfg(windows)]
#![allow(non_snake_case)]

mod config;
mod convert;
mod device;
mod driver;
mod iface;
mod log;

use core::ffi::c_void;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicIsize, Ordering};

use sw5_win::reg::{HKEY_LOCAL_MACHINE, Key, View, delete_tree};
use windows::Win32::Foundation::{
    CLASS_E_CLASSNOTAVAILABLE, CLASS_E_NOAGGREGATION, E_POINTER, E_UNEXPECTED, HMODULE, S_FALSE, S_OK,
};
use windows::Win32::System::Com::{IClassFactory, IClassFactory_Impl};
use windows::Win32::System::LibraryLoader::{
    GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS, GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT, GetModuleFileNameW,
    GetModuleHandleExW,
};
use windows_core::{BOOL, GUID, HRESULT, IUnknown, Interface, PCWSTR, Ref, implement};

pub use driver::EffectDriver;
pub use iface::IDirectInputEffectDriver;
use log::log;

pub const CLSID: GUID = GUID::from_u128(0xEAA22CE3_B98A_4DC8_8D2E_A502FF34B7E4);

static OBJECTS: AtomicIsize = AtomicIsize::new(0);
static LOCKS: AtomicIsize = AtomicIsize::new(0);

/// Cuenta objetos vivos para `DllCanUnloadNow`.
struct ObjectCount;
impl ObjectCount {
    fn new() -> Self {
        OBJECTS.fetch_add(1, Ordering::SeqCst);
        ObjectCount
    }
}
impl Drop for ObjectCount {
    fn drop(&mut self) {
        OBJECTS.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Ejecuta `f` atrapando panics: un panic que cruza `extern "system"` aborta el juego.
fn guard(f: impl FnOnce() -> HRESULT) -> HRESULT {
    catch_unwind(AssertUnwindSafe(f)).unwrap_or_else(|_| {
        log!("panic atrapado en la frontera COM");
        E_UNEXPECTED
    })
}

#[implement(IClassFactory)]
struct Factory {
    _count: ObjectCount,
}

impl IClassFactory_Impl for Factory_Impl {
    fn CreateInstance(
        &self,
        outer: Ref<IUnknown>,
        riid: *const GUID,
        ppv: *mut *mut c_void,
    ) -> windows_core::Result<()> {
        guard(|| unsafe {
            if ppv.is_null() || riid.is_null() {
                return E_POINTER;
            }
            *ppv = core::ptr::null_mut();
            if !outer.is_null() {
                return CLASS_E_NOAGGREGATION;
            }
            let unk: IUnknown = EffectDriver::new().into();
            unk.query(riid, ppv)
        })
        .ok()
    }

    fn LockServer(&self, lock: BOOL) -> windows_core::Result<()> {
        LOCKS.fetch_add(if lock.as_bool() { 1 } else { -1 }, Ordering::SeqCst);
        Ok(())
    }
}

/// # Safety
/// Lo llama COM: `clsid`, `riid` y `ppv` deben ser punteros válidos.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn DllGetClassObject(
    clsid: *const GUID,
    riid: *const GUID,
    ppv: *mut *mut c_void,
) -> HRESULT {
    guard(|| unsafe {
        if clsid.is_null() || riid.is_null() || ppv.is_null() {
            return E_POINTER;
        }
        *ppv = core::ptr::null_mut();
        if *clsid != CLSID {
            return CLASS_E_CLASSNOTAVAILABLE;
        }
        let factory: IClassFactory = Factory { _count: ObjectCount::new() }.into();
        factory.query(riid, ppv)
    })
}

#[unsafe(no_mangle)]
pub extern "system" fn DllCanUnloadNow() -> HRESULT {
    if OBJECTS.load(Ordering::SeqCst) == 0 && LOCKS.load(Ordering::SeqCst) <= 0 { S_OK } else { S_FALSE }
}

/// Ruta de esta DLL.
fn module_path() -> Option<String> {
    let mut h = HMODULE::default();
    unsafe {
        GetModuleHandleExW(
            GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
            PCWSTR(module_path as *const u16),
            &mut h,
        )
        .ok()?;
        let mut buf = [0u16; 1024];
        let n = GetModuleFileNameW(Some(h), &mut buf) as usize;
        (n > 0 && n < buf.len()).then(|| String::from_utf16_lossy(&buf[..n]))
    }
}

const fn own_view() -> View {
    if cfg!(target_pointer_width = "64") { View::Bits64 } else { View::Bits32 }
}

const CLSID_PARENT: &str = r"SOFTWARE\Classes\CLSID";

/// Registra el CLSID en la vista del registro de esta arquitectura. Las claves OEM
/// del volante las escribe el instalador (`sw5 install`), no esta función.
#[unsafe(no_mangle)]
pub extern "system" fn DllRegisterServer() -> HRESULT {
    guard(|| {
        let Some(path) = module_path() else { return E_UNEXPECTED };
        let r = (|| -> windows_core::Result<()> {
            let k = Key::create(HKEY_LOCAL_MACHINE, &format!(r"{CLSID_PARENT}\{}", sw5_win::CLSID_STR), own_view())?;
            k.set_sz("", "Speed Wheel 5 Pro force feedback driver (Rust)")?;
            let s = k.subkey("InprocServer32")?;
            s.set_sz("", &path)?;
            s.set_sz("ThreadingModel", "Both")?;
            Ok(())
        })();
        r.map_or_else(|e| e.code(), |_| S_OK)
    })
}

#[unsafe(no_mangle)]
pub extern "system" fn DllUnregisterServer() -> HRESULT {
    guard(|| {
        delete_tree(HKEY_LOCAL_MACHINE, CLSID_PARENT, sw5_win::CLSID_STR, own_view())
            .map_or_else(|e| e.code(), |_| S_OK)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clsid_constants_agree() {
        assert_eq!(format!("{{{CLSID:?}}}"), sw5_win::CLSID_STR);
    }
}

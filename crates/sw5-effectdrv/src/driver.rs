//! Implementación de `IDirectInputEffectDriver`.
//!
//! DirectInput identifica cada volante por su *external ID* (`dwID`); una misma
//! instancia del driver puede atender a varios.

#![allow(non_snake_case)]

use core::ffi::c_void;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use sw5_core::{EngineError, INFINITE};
use windows::Win32::Devices::HumanInterfaceDevice::{
    DIDEVICESTATE, DIDRIVERVERSIONS, DIEFFECT, DIEFFESCAPE, DIEGES_PLAYING, DIEP_NODOWNLOAD, DIEP_START,
    DIERR_DEVICEFULL, DIERR_INVALIDPARAM, DIERR_NOTINITIALIZED, DIERR_UNSUPPORTED, DIES_SOLO, DIGFFS_ACTUATORSOFF,
    DIGFFS_ACTUATORSON, DIGFFS_EMPTY, DIGFFS_PAUSED, DIGFFS_POWERON, DIGFFS_STOPPED, DIHIDFFINITINFO, DISFFC_CONTINUE,
    DISFFC_PAUSE, DISFFC_RESET, DISFFC_SETACTUATORSOFF, DISFFC_SETACTUATORSON, DISFFC_STOPALL,
};
use windows::Win32::Foundation::{E_POINTER, S_OK};
use windows_core::{HRESULT, implement};

use crate::config::{self, Config};
use crate::convert::effect_from_di;
use crate::device::{Device, Shared};
use crate::iface::{IDirectInputEffectDriver, IDirectInputEffectDriver_Impl};
use crate::log::log;
use crate::{ObjectCount, guard};

const INVALID: HRESULT = HRESULT(DIERR_INVALIDPARAM);

#[implement(IDirectInputEffectDriver)]
pub struct EffectDriver {
    devices: Mutex<HashMap<u32, Device>>,
    cfg: Config,
    _count: ObjectCount,
}

impl EffectDriver {
    pub fn new() -> Self {
        let cfg = config::load();
        if let Some(f) = &cfg.log_file {
            crate::log::open_file(f);
        }
        log!("EffectDriver creado ({cfg:?})");
        EffectDriver { devices: Mutex::new(HashMap::new()), cfg, _count: ObjectCount::new() }
    }

    fn devices(&self) -> std::sync::MutexGuard<'_, HashMap<u32, Device>> {
        self.devices.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Estado del dispositivo `id`, sin retener el lock del mapa.
    fn shared(&self, id: u32) -> Result<Arc<Shared>, HRESULT> {
        self.devices().get(&id).map(Device::shared).ok_or(DIERR_NOTINITIALIZED)
    }

    fn with_engine(&self, id: u32, f: impl FnOnce(&Shared, &mut sw5_core::Engine) -> HRESULT) -> HRESULT {
        match self.shared(id) {
            Ok(sh) => {
                let hr = {
                    let mut e = sh.engine();
                    f(&sh, &mut e)
                };
                sh.wake();
                hr
            }
            Err(hr) => hr,
        }
    }

    fn verbose(&self) -> bool {
        self.cfg.verbose
    }
}

impl Default for EffectDriver {
    fn default() -> Self {
        Self::new()
    }
}

fn engine_err(e: EngineError) -> HRESULT {
    match e {
        EngineError::NotFound => INVALID,
        EngineError::DeviceFull => HRESULT(DIERR_DEVICEFULL),
    }
}

fn wide_to_vec(p: windows_core::PWSTR) -> Vec<u16> {
    let s = unsafe { p.as_wide() };
    s.iter().copied().chain(Some(0)).collect()
}

impl IDirectInputEffectDriver_Impl for EffectDriver_Impl {
    unsafe fn DeviceID(
        &self,
        di_version: u32,
        external_id: u32,
        begin: u32,
        internal_id: u32,
        reserved: *mut c_void,
    ) -> HRESULT {
        guard(|| {
            log!("DeviceID(version={di_version:#x}, external={external_id}, begin={begin}, internal={internal_id})");
            if begin == 0 {
                let old = self.devices().remove(&external_id);
                drop(old); // detiene y espera al hilo fuera del lock
                return S_OK;
            }
            let info = unsafe { (reserved as *const DIHIDFFINITINFO).as_ref() }
                .filter(|i| i.dwSize as usize >= size_of::<DIHIDFFINITINFO>() && !i.pwszDeviceInterface.is_null());
            let path = match info {
                Some(i) => wide_to_vec(i.pwszDeviceInterface),
                None => match sw5_win::hid::find_wheels().ok().and_then(|w| w.into_iter().next()) {
                    Some(d) => d.path,
                    None => return HRESULT(DIERR_NOTINITIALIZED.0),
                },
            };
            log!("  interfaz: {}", sw5_win::from_wide(&path));
            if self.devices().get(&external_id).is_some_and(|d| d.path == path) {
                return S_OK;
            }
            match Device::open(path, &self.cfg) {
                Ok(dev) => {
                    let old = self.devices().insert(external_id, dev);
                    drop(old);
                    S_OK
                }
                Err(hr) => {
                    log!("  no se pudo abrir: {hr:?}");
                    hr
                }
            }
        })
    }

    unsafe fn GetVersions(&self, vers: *mut DIDRIVERVERSIONS) -> HRESULT {
        guard(|| {
            let Some(v) = (unsafe { vers.as_mut() }) else { return E_POINTER };
            let hw = self.devices().values().next().map_or(0, |d| d.version as u32);
            v.dwFirmwareRevision = hw;
            v.dwHardwareRevision = hw;
            v.dwFFDriverVersion = 0x0001_0000;
            S_OK
        })
    }

    unsafe fn Escape(&self, _id: u32, _effect: u32, _esc: *mut DIEFFESCAPE) -> HRESULT {
        HRESULT(DIERR_UNSUPPORTED)
    }

    unsafe fn SetGain(&self, id: u32, gain: u32) -> HRESULT {
        guard(|| {
            if self.verbose() {
                log!("SetGain({id}, {gain})");
            }
            self.with_engine(id, |_, e| {
                e.set_gain(gain);
                S_OK
            })
        })
    }

    unsafe fn SendForceFeedbackCommand(&self, id: u32, command: u32) -> HRESULT {
        guard(|| {
            log!("SendForceFeedbackCommand({id}, {command:#x})");
            self.with_engine(id, |sh, e| {
                let now = sh.now_us();
                match command {
                    DISFFC_RESET => e.reset(),
                    DISFFC_STOPALL => e.stop_all(),
                    DISFFC_PAUSE => e.pause(now),
                    DISFFC_CONTINUE => e.resume(now),
                    DISFFC_SETACTUATORSON => e.set_actuators(true),
                    DISFFC_SETACTUATORSOFF => e.set_actuators(false),
                    _ => return INVALID,
                }
                S_OK
            })
        })
    }

    unsafe fn GetForceFeedbackState(&self, id: u32, state: *mut DIDEVICESTATE) -> HRESULT {
        guard(|| {
            let Some(out) = (unsafe { state.as_mut() }) else { return E_POINTER };
            self.with_engine(id, |sh, e| {
                let s = e.state(sh.now_us());
                let mut flags = DIGFFS_POWERON;
                flags |= if s.actuators_on { DIGFFS_ACTUATORSON } else { DIGFFS_ACTUATORSOFF };
                if s.empty {
                    flags |= DIGFFS_EMPTY;
                }
                if s.stopped {
                    flags |= DIGFFS_STOPPED;
                }
                if s.paused {
                    flags |= DIGFFS_PAUSED;
                }
                out.dwState = flags;
                out.dwLoad = s.load;
                S_OK
            })
        })
    }

    unsafe fn DownloadEffect(
        &self,
        id: u32,
        effect_id: u32,
        effect: *mut u32,
        params: *const DIEFFECT,
        flags: u32,
    ) -> HRESULT {
        guard(|| {
            let (Some(handle), Some(p)) = (unsafe { effect.as_mut() }, unsafe { params.as_ref() }) else {
                return E_POINTER;
            };
            let eff = match unsafe { effect_from_di(effect_id, p) } {
                Ok(e) => e,
                Err(hr) => {
                    log!("DownloadEffect: parámetros inválidos (effect_id={effect_id:#x})");
                    return hr;
                }
            };
            if self.verbose() {
                log!("DownloadEffect({id}, id={effect_id:#x}, handle={}, flags={flags:#x}) {eff:?}", *handle);
            }
            if flags & DIEP_NODOWNLOAD != 0 {
                return S_OK; // solo validar
            }
            self.with_engine(id, |sh, e| {
                let h = if *handle == 0 {
                    match e.create(eff) {
                        Ok(h) => {
                            *handle = h;
                            h
                        }
                        Err(err) => return engine_err(err),
                    }
                } else {
                    if let Err(err) = e.update(*handle, eff) {
                        return engine_err(err);
                    }
                    *handle
                };
                if flags & DIEP_START != 0 {
                    let _ = e.start(h, 1, false, sh.now_us());
                }
                S_OK
            })
        })
    }

    unsafe fn DestroyEffect(&self, id: u32, effect: u32) -> HRESULT {
        guard(|| {
            if self.verbose() {
                log!("DestroyEffect({id}, {effect})");
            }
            self.with_engine(id, |_, e| e.destroy(effect).map_or_else(engine_err, |_| S_OK))
        })
    }

    unsafe fn StartEffect(&self, id: u32, effect: u32, mode: u32, count: u32) -> HRESULT {
        guard(|| {
            if self.verbose() {
                log!("StartEffect({id}, {effect}, mode={mode:#x}, count={count})");
            }
            let iterations = if count == 0 { 1 } else { count };
            self.with_engine(id, |sh, e| {
                e.start(effect, iterations, mode & DIES_SOLO != 0, sh.now_us()).map_or_else(engine_err, |_| S_OK)
            })
        })
    }

    unsafe fn StopEffect(&self, id: u32, effect: u32) -> HRESULT {
        guard(|| {
            if self.verbose() {
                log!("StopEffect({id}, {effect})");
            }
            self.with_engine(id, |_, e| e.stop(effect).map_or_else(engine_err, |_| S_OK))
        })
    }

    unsafe fn GetEffectStatus(&self, id: u32, effect: u32, status: *mut u32) -> HRESULT {
        guard(|| {
            let Some(out) = (unsafe { status.as_mut() }) else { return E_POINTER };
            self.with_engine(id, |sh, e| match e.is_playing(effect, sh.now_us()) {
                Ok(playing) => {
                    *out = if playing { DIEGES_PLAYING } else { 0 };
                    S_OK
                }
                Err(err) => engine_err(err),
            })
        })
    }
}

// INFINITE de Win32 y del core deben coincidir: DirectInput pasa 0xFFFFFFFF.
const _: () = assert!(INFINITE == u32::MAX);

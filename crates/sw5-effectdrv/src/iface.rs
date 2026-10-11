//! Declaración propia de `IDirectInputEffectDriver` (dinputd.h).
//!
//! windows-rs trae una versión con `Result<()>`, pero no permite devolver códigos
//! de éxito como `S_FALSE` o `DI_DOWNLOADSKIPPED`, y declara el `DIEFFECT` como
//! mutable. El orden de la vtable está verificado contra
//! `Windows Kits\10\Include\10.0.26100.0\shared\dinputd.h`.

#![allow(non_snake_case)]

use core::ffi::c_void;

use windows::Win32::Devices::HumanInterfaceDevice::{DIDEVICESTATE, DIDRIVERVERSIONS, DIEFFECT, DIEFFESCAPE};
use windows_core::{HRESULT, IUnknown, IUnknown_Vtbl, interface};

#[interface("02538130-898F-11D0-9AD0-00A0C9A06E35")]
pub unsafe trait IDirectInputEffectDriver: IUnknown {
    pub unsafe fn DeviceID(
        &self,
        di_version: u32,
        external_id: u32,
        begin: u32,
        internal_id: u32,
        reserved: *mut c_void,
    ) -> HRESULT;
    pub unsafe fn GetVersions(&self, vers: *mut DIDRIVERVERSIONS) -> HRESULT;
    pub unsafe fn Escape(&self, id: u32, effect: u32, esc: *mut DIEFFESCAPE) -> HRESULT;
    pub unsafe fn SetGain(&self, id: u32, gain: u32) -> HRESULT;
    pub unsafe fn SendForceFeedbackCommand(&self, id: u32, command: u32) -> HRESULT;
    pub unsafe fn GetForceFeedbackState(&self, id: u32, state: *mut DIDEVICESTATE) -> HRESULT;
    pub unsafe fn DownloadEffect(
        &self,
        id: u32,
        effect_id: u32,
        effect: *mut u32,
        params: *const DIEFFECT,
        flags: u32,
    ) -> HRESULT;
    pub unsafe fn DestroyEffect(&self, id: u32, effect: u32) -> HRESULT;
    pub unsafe fn StartEffect(&self, id: u32, effect: u32, mode: u32, count: u32) -> HRESULT;
    pub unsafe fn StopEffect(&self, id: u32, effect: u32) -> HRESULT;
    pub unsafe fn GetEffectStatus(&self, id: u32, effect: u32, status: *mut u32) -> HRESULT;
}

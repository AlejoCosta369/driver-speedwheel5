//! Piezas específicas de Windows compartidas por el effect driver y la CLI.

#![cfg(windows)]

pub mod hid;
pub mod oem;
pub mod reg;

/// CLSID del effect driver nuevo. Distinto del original
/// ({2940F080-75AE-11d7-BC91-00501a5c3012}) para poder volver atrás.
pub const CLSID_STR: &str = "{EAA22CE3-B98A-4DC8-8D2E-A502FF34B7E4}";

/// Configuración del driver (siempre en la vista de 64 bits del registro, para que
/// la DLL de 32 y la de 64 lean la misma).
pub const CONFIG_KEY: &str = r"SOFTWARE\SpeedWheel5";

pub use sw5_core::{PIDS, VID};

/// Convierte un `&str` en un buffer UTF-16 terminado en NUL.
pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

/// Convierte un buffer UTF-16 (terminado en NUL o no) en `String`.
pub fn from_wide(s: &[u16]) -> String {
    let end = s.iter().position(|&c| c == 0).unwrap_or(s.len());
    String::from_utf16_lossy(&s[..end])
}

//! Log liviano: `OutputDebugString` siempre, y un archivo opcional configurado en el
//! registro (`LogFile`). Nunca se escribe en stdout/stderr: la DLL vive dentro de
//! un juego.

use std::fmt::Arguments;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

use windows::Win32::System::Diagnostics::Debug::OutputDebugStringW;
use windows_core::HSTRING;

static FILE: Mutex<Option<File>> = Mutex::new(None);
static START: OnceLock<Instant> = OnceLock::new();

pub fn open_file(path: &str) {
    if let Ok(f) = OpenOptions::new().create(true).append(true).open(path) {
        *FILE.lock().unwrap_or_else(|e| e.into_inner()) = Some(f);
    }
}

pub fn write(args: Arguments) {
    let t = START.get_or_init(Instant::now).elapsed();
    let bits = if cfg!(target_pointer_width = "64") { 64 } else { 32 };
    let line = format!("[sw5 pid={} x{bits} +{}.{:03}s] {args}\n", std::process::id(), t.as_secs(), t.subsec_millis());
    unsafe { OutputDebugStringW(&HSTRING::from(line.as_str())) };
    if let Some(f) = FILE.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
        let _ = f.write_all(line.as_bytes());
    }
}

macro_rules! log {
    ($($t:tt)*) => { $crate::log::write(format_args!($($t)*)) };
}
pub(crate) use log;

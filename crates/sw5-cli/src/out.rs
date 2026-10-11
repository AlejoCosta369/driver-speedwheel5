//! Salida de la CLI. Cuando el comando corre elevado (ventana oculta), además se
//! escribe en un archivo que el proceso original muestra al terminar.

use std::fmt::Arguments;
use std::fs::File;
use std::io::Write;
use std::path::Path;
use std::sync::Mutex;

static SINK: Mutex<Option<File>> = Mutex::new(None);

pub fn tee_to(path: &Path) {
    if let Ok(f) = File::create(path) {
        *SINK.lock().unwrap() = Some(f);
    }
}

pub fn emit(args: Arguments) {
    println!("{args}");
    if let Some(f) = SINK.lock().unwrap().as_mut() {
        let _ = writeln!(f, "{args}");
    }
}

macro_rules! say {
    ($($t:tt)*) => { $crate::out::emit(format_args!($($t)*)) };
}
pub(crate) use say;

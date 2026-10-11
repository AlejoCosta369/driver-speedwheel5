//! Configuración en `HKLM\SOFTWARE\SpeedWheel5` (vista de 64 bits) y, con
//! prioridad, en `HKCU\SOFTWARE\SpeedWheel5` (por usuario, no requiere
//! administrador). Se lee al crear el driver. Todos los valores son opcionales.
//!
//! | Valor        | Tipo  | Default | Significado                                 |
//! |--------------|-------|---------|---------------------------------------------|
//! | Strength     | DWORD | 100     | intensidad global en %                      |
//! | MinLevel     | DWORD | 0       | byte mínimo con el motor encendido          |
//! | Gamma        | DWORD | 100     | curva de respuesta × 100                    |
//! | SwapMotors   | DWORD | 0       | intercambiar motor fuerte y débil           |
//! | TickMs       | DWORD | 4       | período del motor de mezcla (1..=50)        |
//! | KeepaliveMs  | DWORD | 100     | reenvío del report mientras vibra (0 = no)  |
//! | LogFile      | SZ    | —       | archivo de log (además de OutputDebugString) |
//! | LogVerbose   | DWORD | 0       | loguear cada llamada a DownloadEffect etc.  |

use sw5_core::OutputConfig;
use sw5_win::CONFIG_KEY;
use sw5_win::reg::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, Key, View};

#[derive(Debug, Clone)]
pub struct Config {
    pub output: OutputConfig,
    pub tick_ms: u32,
    pub keepalive_ms: u32,
    pub log_file: Option<String>,
    pub verbose: bool,
}

impl Default for Config {
    fn default() -> Self {
        Config { output: OutputConfig::default(), tick_ms: 4, keepalive_ms: 100, log_file: None, verbose: false }
    }
}

pub fn load() -> Config {
    let mut c = Config::default();
    for root in [HKEY_LOCAL_MACHINE, HKEY_CURRENT_USER] {
        if let Ok(Some(k)) = Key::open(root, CONFIG_KEY, View::Bits64, false) {
            apply(&mut c, &k);
        }
    }
    c
}

fn apply(c: &mut Config, k: &Key) {
    let dw = |name: &str| k.get_dword(name).ok().flatten();
    if let Some(v) = dw("Strength") {
        c.output.strength = v.min(100) as f32 / 100.0;
    }
    if let Some(v) = dw("MinLevel") {
        c.output.min_level = v.min(255) as u8;
    }
    if let Some(v) = dw("Gamma") {
        c.output.gamma = (v.clamp(10, 1000)) as f32 / 100.0;
    }
    if let Some(v) = dw("SwapMotors") {
        c.output.swap_motors = v != 0;
    }
    if let Some(v) = dw("TickMs") {
        c.tick_ms = v.clamp(1, 50);
    }
    if let Some(v) = dw("KeepaliveMs") {
        c.keepalive_ms = v;
    }
    if let Some(v) = dw("LogVerbose") {
        c.verbose = v != 0;
    }
    if let Some(f) = k.get_sz("LogFile").ok().flatten() {
        c.log_file = Some(f).filter(|s| !s.is_empty());
    }
}

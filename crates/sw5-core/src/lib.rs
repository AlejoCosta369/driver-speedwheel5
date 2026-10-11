//! Núcleo del driver del Speed Wheel 5 Pro, independiente de la plataforma.
//!
//! El volante tiene **dos motores de vibración** (confirmado con el hardware), no un
//! motor de torsión: el motor A (byte 3 del report) es el fuerte/grave y el B
//! (byte 4) el suave/agudo. Este crate traduce efectos con la semántica de
//! DirectInput a dos niveles de vibración:
//!
//! * [`effect`]: parámetros de un efecto y su fuerza en función del tiempo.
//! * [`engine`]: efectos descargados, reproducción, pausa, ganancia y mezcla.
//! * [`output`]: conversión de niveles a bytes y codificación del output report.
//!
//! Los tiempos son microsegundos de un reloj monotónico que provee quien lo usa.

#![no_std]

extern crate alloc;
#[cfg(test)]
extern crate std;

pub mod effect;
pub mod engine;
pub mod output;

pub use effect::{ConditionKind, Effect, Envelope, Kind, Routing, Waveform};
pub use engine::{DeviceState, Engine, EngineError, Levels};
pub use output::{OutputConfig, REPORT_LEN, STOP_REPORT, encode_report};

/// Vendor ID USB de Arkino Corporation (fabricante del hardware).
pub const VID: u16 = 0x1A5C;
/// Product IDs conocidos del Speed Wheel 5 Pro.
pub const PIDS: [u16; 2] = [0x3112, 0x3012];

/// Magnitud nominal máxima de DirectInput (`DI_FFNOMINALMAX`).
pub const NOMINAL_MAX: i32 = 10_000;
/// Duración o cantidad de iteraciones infinitas (`INFINITE` de Win32).
pub const INFINITE: u32 = u32::MAX;

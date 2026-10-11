//! Conversión de niveles de motor a bytes y codificación del output report.
//!
//! Report confirmado con el hardware (HID: ID 0 + cuatro campos de 8 bits en la
//! página LED, usages 0x43..0x46):
//!
//! | byte | contenido                                  |
//! |------|--------------------------------------------|
//! | 0    | Report ID (0)                              |
//! | 1    | 0                                          |
//! | 2    | flags (0; el original los usaba al apagar) |
//! | 3    | motor A, fuerte / grave (0..255)           |
//! | 4    | motor B, suave / agudo (0..255)            |

use libm::{powf, roundf};

use crate::Levels;

pub const REPORT_LEN: usize = 5;
/// Apaga los dos motores.
pub const STOP_REPORT: [u8; REPORT_LEN] = [0; REPORT_LEN];

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OutputConfig {
    /// Intensidad global configurable por el usuario, 0..=1.
    pub strength: f32,
    /// Por debajo de este nivel (0..=1) el motor queda apagado.
    pub deadzone: f32,
    /// Nivel mínimo en bytes cuando el motor está encendido: los motores de
    /// vibración no arrancan con tensiones muy bajas.
    pub min_level: u8,
    pub max_level: u8,
    /// Curva de respuesta: 1 = lineal; < 1 realza las fuerzas débiles.
    pub gamma: f32,
    pub swap_motors: bool,
}

impl Default for OutputConfig {
    fn default() -> Self {
        OutputConfig { strength: 1.0, deadzone: 0.005, min_level: 0, max_level: 255, gamma: 1.0, swap_motors: false }
    }
}

impl OutputConfig {
    /// Nivel 0..=1 → byte del motor.
    pub fn to_byte(&self, x: f32) -> u8 {
        let x = (x * self.strength).clamp(0.0, 1.0);
        if x < self.deadzone || x == 0.0 {
            return 0;
        }
        let y = if self.gamma == 1.0 { x } else { powf(x, self.gamma) };
        let (lo, hi) = (self.min_level as f32, self.max_level.max(self.min_level) as f32);
        let v = roundf(lo + (hi - lo) * y);
        (v as u8).max(1)
    }
}

/// Arma el output report para los niveles dados.
pub fn encode_report(levels: Levels, cfg: &OutputConfig) -> [u8; REPORT_LEN] {
    let (mut a, mut b) = (cfg.to_byte(levels.strong), cfg.to_byte(levels.weak));
    if cfg.swap_motors {
        core::mem::swap(&mut a, &mut b);
    }
    [0, 0, 0, a, b]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_motor_bytes() {
        let cfg = OutputConfig::default();
        assert_eq!(encode_report(Levels { strong: 1.0, weak: 0.0 }, &cfg), [0, 0, 0, 255, 0]);
        assert_eq!(encode_report(Levels { strong: 0.0, weak: 0.5 }, &cfg), [0, 0, 0, 0, 128]);
        assert_eq!(encode_report(Levels::OFF, &cfg), STOP_REPORT);
    }

    #[test]
    fn swap_strength_deadzone_and_min_level() {
        let cfg = OutputConfig { swap_motors: true, ..Default::default() };
        assert_eq!(encode_report(Levels { strong: 1.0, weak: 0.0 }, &cfg), [0, 0, 0, 0, 255]);

        let cfg = OutputConfig { strength: 0.5, ..Default::default() };
        assert_eq!(cfg.to_byte(1.0), 128);

        let cfg = OutputConfig { min_level: 60, ..Default::default() };
        assert_eq!(cfg.to_byte(0.001), 0); // dentro de la zona muerta
        assert_eq!(cfg.to_byte(0.01), 62);
        assert_eq!(cfg.to_byte(1.0), 255);
    }
}

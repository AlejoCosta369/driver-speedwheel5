//! Parámetros de un efecto y cálculo de su fuerza en el tiempo.

use alloc::vec::Vec;
use core::f32::consts::TAU;

use libm::{cosf, fabsf, floorf, sinf};

use crate::INFINITE;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Waveform {
    Square,
    Sine,
    Triangle,
    SawtoothUp,
    SawtoothDown,
}

impl Waveform {
    /// Valor en [-1, 1] para la posición `p` ∈ [0, 1) dentro del período.
    /// Con fase 0, seno y triángulo arrancan en 0 subiendo.
    pub fn sample(self, p: f32) -> f32 {
        match self {
            Waveform::Square => {
                if p < 0.5 {
                    1.0
                } else {
                    -1.0
                }
            }
            Waveform::Sine => sinf(TAU * p),
            Waveform::Triangle => {
                if p < 0.25 {
                    4.0 * p
                } else if p < 0.75 {
                    2.0 - 4.0 * p
                } else {
                    4.0 * p - 4.0
                }
            }
            Waveform::SawtoothUp => 2.0 * p - 1.0,
            Waveform::SawtoothDown => 1.0 - 2.0 * p,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConditionKind {
    Spring,
    Damper,
    Inertia,
    Friction,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Kind {
    Constant {
        magnitude: i32,
    },
    Ramp {
        start: i32,
        end: i32,
    },
    Periodic {
        waveform: Waveform,
        magnitude: u32,
        offset: i32,
        /// Centésimas de grado (0..36000).
        phase: u32,
        period_us: u32,
    },
    /// Las condiciones necesitan un motor de torsión. Con dos motores de vibración
    /// se aceptan (los juegos de carreras las crean siempre) pero no producen fuerza.
    Condition(ConditionKind),
    Custom {
        /// Muestras entrelazadas por canal; se usa el canal 0.
        samples: Vec<i32>,
        channels: u32,
        sample_period_us: u32,
    },
}

/// Envolvente de DirectInput: niveles absolutos (0..=10000) y tiempos en µs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Envelope {
    pub attack_level: u32,
    pub attack_time_us: u32,
    pub fade_level: u32,
    pub fade_time_us: u32,
}

/// Cuánto de la fuerza de un efecto va a cada motor, en [0, 1].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Routing {
    pub strong: f32,
    pub weak: f32,
}

impl Routing {
    pub const BOTH: Routing = Routing { strong: 1.0, weak: 1.0 };

    /// Convención clásica de los gamepads rumble con DirectInput: la componente X
    /// va al motor fuerte y la Y al débil, normalizadas por la mayor de las dos.
    pub fn from_cartesian(x: f32, y: f32) -> Routing {
        let (ax, ay) = (fabsf(x), fabsf(y));
        let m = ax.max(ay);
        if m == 0.0 {
            return Routing::BOTH;
        }
        Routing { strong: ax / m, weak: ay / m }
    }

    /// Ángulo polar de DirectInput en centésimas de grado: 0 = norte (−Y),
    /// 9000 = este (+X).
    pub fn from_polar(angle: i32) -> Routing {
        let rad = angle.rem_euclid(36000) as f32 / 36000.0 * TAU;
        let (x, y) = (sinf(rad), -cosf(rad));
        // Redondear el ruido de punto flotante (sin(π) ≠ 0 exacto).
        let clean = |v: f32| if fabsf(v) < 1e-4 { 0.0 } else { v };
        Routing::from_cartesian(clean(x), clean(y))
    }

    /// Primer ángulo esférico de DirectInput en centésimas de grado, medido desde
    /// +X hacia +Y.
    pub fn from_spherical(angle: i32) -> Routing {
        let rad = angle.rem_euclid(36000) as f32 / 36000.0 * TAU;
        let clean = |v: f32| if fabsf(v) < 1e-4 { 0.0 } else { v };
        Routing::from_cartesian(clean(cosf(rad)), clean(sinf(rad)))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Effect {
    pub kind: Kind,
    /// Duración de una iteración en µs; [`INFINITE`] = sin fin.
    pub duration_us: u32,
    pub start_delay_us: u32,
    /// Ganancia del efecto, 0..=10000.
    pub gain: u32,
    pub envelope: Option<Envelope>,
    pub routing: Routing,
}

impl Effect {
    pub fn new(kind: Kind) -> Self {
        Effect { kind, duration_us: INFINITE, start_delay_us: 0, gain: 10_000, envelope: None, routing: Routing::BOTH }
    }

    /// Fuerza con signo, en unidades de DirectInput (±10000 nominal), a `t_us` del
    /// comienzo de la iteración actual. No incluye ganancias.
    pub fn force_at(&self, t_us: u64) -> f32 {
        match &self.kind {
            Kind::Constant { magnitude } => self.apply_envelope(*magnitude as f32, t_us),
            Kind::Ramp { start, end } => {
                let v = if self.duration_us == INFINITE || self.duration_us == 0 {
                    *start as f32
                } else {
                    let k = (t_us as f32 / self.duration_us as f32).min(1.0);
                    *start as f32 + (*end as f32 - *start as f32) * k
                };
                self.apply_envelope(v, t_us)
            }
            Kind::Periodic { waveform, magnitude, offset, phase, period_us } => {
                let pos = if *period_us == 0 { 0.0 } else { (t_us % *period_us as u64) as f32 / *period_us as f32 };
                let p = wrap01(pos + (*phase % 36000) as f32 / 36000.0);
                let mag = self.envelope_level(*magnitude as f32, t_us);
                *offset as f32 + mag * waveform.sample(p)
            }
            Kind::Condition(_) => 0.0,
            Kind::Custom { samples, channels, sample_period_us } => {
                let ch = (*channels).max(1) as usize;
                let frames = samples.len() / ch;
                if frames == 0 {
                    return 0.0;
                }
                let sp = if *sample_period_us == 0 { 1000 } else { *sample_period_us as u64 };
                let idx = ((t_us / sp) as usize % frames) * ch;
                self.apply_envelope(samples[idx] as f32, t_us)
            }
        }
    }

    /// Nivel absoluto tras la envolvente, para un nivel de sostenido `sustain` ≥ 0.
    fn envelope_level(&self, sustain: f32, t_us: u64) -> f32 {
        let Some(e) = &self.envelope else {
            return sustain;
        };
        let t = t_us as f32;
        if e.attack_time_us > 0 && t < e.attack_time_us as f32 {
            let a = e.attack_level as f32;
            return a + (sustain - a) * t / e.attack_time_us as f32;
        }
        if self.duration_us != INFINITE && e.fade_time_us > 0 {
            let d = self.duration_us as f32;
            let f = e.fade_time_us as f32;
            if t > d - f {
                let fl = e.fade_level as f32;
                return fl + (sustain - fl) * (d - t).max(0.0) / f;
            }
        }
        sustain
    }

    /// Aplica la envolvente a la magnitud conservando el signo.
    fn apply_envelope(&self, v: f32, t_us: u64) -> f32 {
        let l = self.envelope_level(fabsf(v), t_us);
        if v < 0.0 { -l } else { l }
    }
}

fn wrap01(x: f32) -> f32 {
    x - floorf(x)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32) -> bool {
        fabsf(a - b) < 1.0
    }

    #[test]
    fn waveforms_span_unit_range() {
        for w in [Waveform::Square, Waveform::Sine, Waveform::Triangle, Waveform::SawtoothUp, Waveform::SawtoothDown] {
            for i in 0..100 {
                let v = w.sample(i as f32 / 100.0);
                assert!((-1.0..=1.0).contains(&v), "{w:?} {v}");
            }
        }
        assert_eq!(Waveform::Triangle.sample(0.25), 1.0);
        assert_eq!(Waveform::Triangle.sample(0.75), -1.0);
        // El original (jsdriver) llegaba a -3 en la segunda mitad del triángulo.
        assert!(Waveform::Triangle.sample(0.9) > -1.0);
    }

    #[test]
    fn constant_keeps_sign() {
        let e = Effect::new(Kind::Constant { magnitude: -5000 });
        assert_eq!(e.force_at(0), -5000.0);
    }

    #[test]
    fn ramp_interpolates_over_duration() {
        let mut e = Effect::new(Kind::Ramp { start: -10000, end: 10000 });
        e.duration_us = 1_000_000;
        assert!(close(e.force_at(0), -10000.0));
        assert!(close(e.force_at(500_000), 0.0));
        assert!(close(e.force_at(1_000_000), 10000.0));
    }

    #[test]
    fn periodic_uses_phase_and_offset() {
        let e = Effect::new(Kind::Periodic {
            waveform: Waveform::Sine,
            magnitude: 10000,
            offset: 1000,
            phase: 9000, // 90°: arranca en el máximo
            period_us: 100_000,
        });
        assert!(close(e.force_at(0), 11000.0));
        assert!(close(e.force_at(50_000), 1000.0 - 10000.0));
    }

    #[test]
    fn envelope_attack_and_fade() {
        let mut e = Effect::new(Kind::Constant { magnitude: 10000 });
        e.duration_us = 1_000_000;
        e.envelope =
            Some(Envelope { attack_level: 0, attack_time_us: 100_000, fade_level: 2000, fade_time_us: 200_000 });
        assert!(close(e.force_at(0), 0.0));
        assert!(close(e.force_at(50_000), 5000.0));
        assert!(close(e.force_at(500_000), 10000.0));
        assert!(close(e.force_at(900_000), 6000.0));
        assert!(close(e.force_at(1_000_000), 2000.0));
    }

    #[test]
    fn custom_cycles_samples() {
        let e =
            Effect::new(Kind::Custom { samples: alloc::vec![100, 200, 300], channels: 1, sample_period_us: 10_000 });
        assert_eq!(e.force_at(0), 100.0);
        assert_eq!(e.force_at(15_000), 200.0);
        assert_eq!(e.force_at(35_000), 100.0);
    }

    #[test]
    fn routing_conventions() {
        assert_eq!(Routing::from_cartesian(10000.0, 0.0), Routing { strong: 1.0, weak: 0.0 });
        assert_eq!(Routing::from_cartesian(0.0, -3.0), Routing { strong: 0.0, weak: 1.0 });
        assert_eq!(Routing::from_cartesian(0.0, 0.0), Routing::BOTH);
        assert_eq!(Routing::from_polar(9000), Routing { strong: 1.0, weak: 0.0 });
        assert_eq!(Routing::from_polar(18000), Routing { strong: 0.0, weak: 1.0 });
        let diag = Routing::from_polar(4500);
        assert!(fabsf(diag.strong - 1.0) < 1e-3 && fabsf(diag.weak - 1.0) < 1e-3);
    }
}

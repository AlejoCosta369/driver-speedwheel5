//! Efectos force feedback de Linux (`struct ff_effect` de `linux/input.h`) →
//! [`sw5_core::Effect`]. No depende de Linux: se testea en cualquier plataforma.
//!
//! Unidades de Linux: niveles `i16` (±0x7FFF), rumble `u16` (0..0xFFFF), tiempos
//! en milisegundos, fase `u16` (0..0xFFFF = una vuelta) y `replay.length = 0`
//! significa "sin fin" (como en `ff-memless`).

use sw5_core::{ConditionKind, Effect, Envelope, INFINITE, Kind, NOMINAL_MAX, Routing, Waveform};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LinuxEnvelope {
    pub attack_length_ms: u16,
    pub attack_level: u16,
    pub fade_length_ms: u16,
    pub fade_level: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinuxKind {
    /// `FF_RUMBLE`: corresponde exactamente a los dos motores del volante.
    Rumble {
        strong: u16,
        weak: u16,
    },
    Constant {
        level: i16,
        envelope: LinuxEnvelope,
    },
    Ramp {
        start: i16,
        end: i16,
        envelope: LinuxEnvelope,
    },
    Periodic {
        waveform: Waveform,
        period_ms: u16,
        magnitude: i16,
        offset: i16,
        phase: u16,
        envelope: LinuxEnvelope,
    },
    Condition(ConditionKind),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LinuxEffect {
    pub kind: LinuxKind,
    pub length_ms: u16,
    pub delay_ms: u16,
}

fn level(v: i16) -> i32 {
    (v as i32 * NOMINAL_MAX / 0x7FFF).clamp(-NOMINAL_MAX, NOMINAL_MAX)
}

fn envelope_level(v: u16) -> u32 {
    (v as u32 * NOMINAL_MAX as u32 / 0x7FFF).min(NOMINAL_MAX as u32)
}

fn envelope(e: &LinuxEnvelope) -> Option<Envelope> {
    if *e == LinuxEnvelope::default() {
        return None;
    }
    Some(Envelope {
        attack_level: envelope_level(e.attack_level),
        attack_time_us: e.attack_length_ms as u32 * 1000,
        fade_level: envelope_level(e.fade_level),
        fade_time_us: e.fade_length_ms as u32 * 1000,
    })
}

/// Linux no tiene concepto de "ganancia por efecto": queda en 100 %. La
/// ganancia global llega aparte (`FF_GAIN`).
pub fn to_core(e: &LinuxEffect) -> Effect {
    let mut routing = Routing::BOTH;
    let mut env = None;
    let kind = match e.kind {
        LinuxKind::Rumble { strong, weak } => {
            let s = strong as f32 / u16::MAX as f32;
            let w = weak as f32 / u16::MAX as f32;
            let m = s.max(w);
            if m > 0.0 {
                routing = Routing { strong: s / m, weak: w / m };
            }
            Kind::Constant { magnitude: (m * NOMINAL_MAX as f32).round() as i32 }
        }
        LinuxKind::Constant { level: l, envelope: ev } => {
            env = envelope(&ev);
            Kind::Constant { magnitude: level(l) }
        }
        LinuxKind::Ramp { start, end, envelope: ev } => {
            env = envelope(&ev);
            Kind::Ramp { start: level(start), end: level(end) }
        }
        LinuxKind::Periodic { waveform, period_ms, magnitude, offset, phase, envelope: ev } => {
            env = envelope(&ev);
            Kind::Periodic {
                waveform,
                magnitude: level(magnitude).unsigned_abs(),
                offset: level(offset),
                phase: (phase as u32 * 36_000) >> 16,
                period_us: period_ms as u32 * 1000,
            }
        }
        LinuxKind::Condition(c) => Kind::Condition(c),
    };
    Effect {
        kind,
        duration_us: if e.length_ms == 0 { INFINITE } else { e.length_ms as u32 * 1000 },
        start_delay_us: e.delay_ms as u32 * 1000,
        gain: 10_000,
        envelope: env,
        routing,
    }
}

/// `FF_GAIN` (0..0xFFFF) → ganancia del motor (0..10000).
pub fn gain(value: i32) -> u32 {
    (value.clamp(0, 0xFFFF) as u32 * 10_000) / 0xFFFF
}

#[cfg(test)]
mod tests {
    use super::*;
    use sw5_core::Engine;

    fn effect(kind: LinuxKind, length_ms: u16) -> LinuxEffect {
        LinuxEffect { kind, length_ms, delay_ms: 0 }
    }

    #[test]
    fn rumble_maps_to_each_motor() {
        let e = to_core(&effect(LinuxKind::Rumble { strong: 0xFFFF, weak: 0x8000 }, 500));
        assert_eq!(e.kind, Kind::Constant { magnitude: 10_000 });
        assert!((e.routing.weak - 0.5).abs() < 0.01 && e.routing.strong == 1.0);
        assert_eq!(e.duration_us, 500_000);

        let mut engine = Engine::new(4);
        let h = engine.create(e).unwrap();
        engine.start(h, 1, false, 0).unwrap();
        let l = engine.mix(1000);
        assert_eq!(l.strong, 1.0);
        assert!((l.weak - 0.5).abs() < 0.01);
    }

    #[test]
    fn weak_only_and_silent_rumble() {
        let e = to_core(&effect(LinuxKind::Rumble { strong: 0, weak: 0xFFFF }, 0));
        assert_eq!(e.routing, Routing { strong: 0.0, weak: 1.0 });
        assert_eq!(e.duration_us, INFINITE);
        let e = to_core(&effect(LinuxKind::Rumble { strong: 0, weak: 0 }, 100));
        assert_eq!(e.kind, Kind::Constant { magnitude: 0 });
    }

    #[test]
    fn levels_phase_and_envelope_are_scaled() {
        let e = to_core(&LinuxEffect {
            kind: LinuxKind::Periodic {
                waveform: Waveform::Sine,
                period_ms: 200,
                magnitude: -0x7FFF,
                offset: 0x4000,
                phase: 0x4000, // un cuarto de vuelta
                envelope: LinuxEnvelope { attack_length_ms: 100, attack_level: 0, fade_length_ms: 0, fade_level: 0 },
            },
            length_ms: 1000,
            delay_ms: 250,
        });
        assert_eq!(
            e.kind,
            Kind::Periodic {
                waveform: Waveform::Sine,
                magnitude: 10_000,
                offset: 5000,
                phase: 9000,
                period_us: 200_000
            }
        );
        assert_eq!(e.start_delay_us, 250_000);
        assert_eq!(e.envelope.unwrap().attack_time_us, 100_000);
        assert_eq!(
            to_core(&effect(LinuxKind::Constant { level: 0x7FFF, envelope: LinuxEnvelope::default() }, 1)).envelope,
            None
        );
    }

    #[test]
    fn gain_range() {
        assert_eq!(gain(0xFFFF), 10_000);
        assert_eq!(gain(0), 0);
        assert_eq!(gain(-5), 0);
    }
}

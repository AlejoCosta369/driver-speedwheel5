//! Efectos descargados, estado de reproducción y mezcla en los dos motores.

use alloc::collections::BTreeMap;

use libm::fabsf;

use crate::{Effect, INFINITE, NOMINAL_MAX};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EngineError {
    /// El handle no corresponde a ningún efecto descargado.
    NotFound,
    /// Se alcanzó el máximo de efectos simultáneos.
    DeviceFull,
}

/// Nivel de cada motor en [0, 1].
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Levels {
    pub strong: f32,
    pub weak: f32,
}

impl Levels {
    pub const OFF: Levels = Levels { strong: 0.0, weak: 0.0 };
}

/// Estado del dispositivo tal como lo reporta `GetForceFeedbackState`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DeviceState {
    /// No hay efectos descargados.
    pub empty: bool,
    /// Ningún efecto se está reproduciendo.
    pub stopped: bool,
    pub paused: bool,
    pub actuators_on: bool,
    /// Porcentaje de memoria de efectos en uso (0..=100).
    pub load: u32,
}

#[derive(Debug, Clone, Copy)]
struct Play {
    /// En el reloj del motor (descuenta el tiempo en pausa).
    start_us: u64,
    iterations: u32,
}

#[derive(Debug, Clone)]
struct Slot {
    effect: Effect,
    play: Option<Play>,
}

/// Fase de un efecto que se está reproduciendo.
enum Phase {
    /// Dentro del start delay: cuenta como reproduciéndose pero no produce fuerza.
    Delay,
    /// Tiempo dentro de la iteración actual.
    Active(u64),
    Finished,
}

pub struct Engine {
    slots: BTreeMap<u32, Slot>,
    next_handle: u32,
    max_effects: usize,
    device_gain: u32,
    actuators_on: bool,
    paused_at: Option<u64>,
    paused_total: u64,
}

impl Engine {
    pub fn new(max_effects: usize) -> Self {
        Engine {
            slots: BTreeMap::new(),
            next_handle: 1,
            max_effects,
            device_gain: 10_000,
            actuators_on: true,
            paused_at: None,
            paused_total: 0,
        }
    }

    /// Reloj interno: el tiempo real menos el acumulado en pausa (congelado mientras dura).
    fn clock(&self, now: u64) -> u64 {
        self.paused_at.unwrap_or(now).saturating_sub(self.paused_total)
    }

    /// Descarga un efecto nuevo; devuelve su handle (nunca 0).
    pub fn create(&mut self, effect: Effect) -> Result<u32, EngineError> {
        if self.slots.len() >= self.max_effects {
            return Err(EngineError::DeviceFull);
        }
        let mut h = self.next_handle;
        while h == 0 || self.slots.contains_key(&h) {
            h = h.wrapping_add(1);
        }
        self.next_handle = h.wrapping_add(1);
        self.slots.insert(h, Slot { effect, play: None });
        Ok(h)
    }

    /// Cambia los parámetros sin reiniciar la reproducción (parámetros dinámicos).
    pub fn update(&mut self, handle: u32, effect: Effect) -> Result<(), EngineError> {
        self.slot_mut(handle)?.effect = effect;
        Ok(())
    }

    pub fn effect(&self, handle: u32) -> Result<&Effect, EngineError> {
        self.slots.get(&handle).map(|s| &s.effect).ok_or(EngineError::NotFound)
    }

    /// (Re)arranca un efecto. `iterations` = [`INFINITE`] repite sin fin.
    /// Con `solo`, detiene todos los demás.
    pub fn start(&mut self, handle: u32, iterations: u32, solo: bool, now: u64) -> Result<(), EngineError> {
        let clock = self.clock(now);
        self.slot_mut(handle)?;
        if solo {
            for s in self.slots.values_mut() {
                s.play = None;
            }
        }
        let slot = self.slot_mut(handle)?;
        slot.play = (iterations != 0).then_some(Play { start_us: clock, iterations });
        Ok(())
    }

    pub fn stop(&mut self, handle: u32) -> Result<(), EngineError> {
        self.slot_mut(handle)?.play = None;
        Ok(())
    }

    pub fn destroy(&mut self, handle: u32) -> Result<(), EngineError> {
        self.slots.remove(&handle).map(|_| ()).ok_or(EngineError::NotFound)
    }

    pub fn is_playing(&self, handle: u32, now: u64) -> Result<bool, EngineError> {
        let slot = self.slots.get(&handle).ok_or(EngineError::NotFound)?;
        let clock = self.clock(now);
        Ok(slot.play.is_some_and(|p| !matches!(phase(&slot.effect, &p, clock), Phase::Finished)))
    }

    pub fn stop_all(&mut self) {
        for s in self.slots.values_mut() {
            s.play = None;
        }
    }

    /// `DISFFC_RESET`: borra todos los efectos y vuelve al estado inicial.
    pub fn reset(&mut self) {
        self.slots.clear();
        self.paused_at = None;
        self.paused_total = 0;
        self.actuators_on = true;
    }

    pub fn pause(&mut self, now: u64) {
        if self.paused_at.is_none() {
            self.paused_at = Some(now);
        }
    }

    pub fn resume(&mut self, now: u64) {
        if let Some(p) = self.paused_at.take() {
            self.paused_total += now.saturating_sub(p);
        }
    }

    pub fn set_actuators(&mut self, on: bool) {
        self.actuators_on = on;
    }

    /// Ganancia global del dispositivo, 0..=10000.
    pub fn set_gain(&mut self, gain: u32) {
        self.device_gain = gain.min(10_000);
    }

    pub fn gain(&self) -> u32 {
        self.device_gain
    }

    pub fn state(&self, now: u64) -> DeviceState {
        let clock = self.clock(now);
        let any_playing = self
            .slots
            .values()
            .any(|s| s.play.is_some_and(|p| !matches!(phase(&s.effect, &p, clock), Phase::Finished)));
        DeviceState {
            empty: self.slots.is_empty(),
            stopped: !any_playing,
            paused: self.paused_at.is_some(),
            actuators_on: self.actuators_on,
            load: (self.slots.len() * 100 / self.max_effects.max(1)) as u32,
        }
    }

    /// Suma la contribución de todos los efectos activos y satura cada motor en 1.0.
    /// Además da por terminados los efectos que agotaron su duración.
    pub fn mix(&mut self, now: u64) -> Levels {
        let clock = self.clock(now);
        let gain = self.device_gain as f32 / 10_000.0;
        let mut out = Levels::OFF;
        for slot in self.slots.values_mut() {
            let Some(play) = slot.play else { continue };
            match phase(&slot.effect, &play, clock) {
                Phase::Finished => slot.play = None,
                Phase::Delay => {}
                Phase::Active(t) => {
                    let e = &slot.effect;
                    let f = fabsf(e.force_at(t)) / NOMINAL_MAX as f32 * (e.gain as f32 / 10_000.0) * gain;
                    out.strong += f * e.routing.strong;
                    out.weak += f * e.routing.weak;
                }
            }
        }
        if self.paused_at.is_some() || !self.actuators_on {
            return Levels::OFF;
        }
        Levels { strong: out.strong.min(1.0), weak: out.weak.min(1.0) }
    }

    fn slot_mut(&mut self, handle: u32) -> Result<&mut Slot, EngineError> {
        self.slots.get_mut(&handle).ok_or(EngineError::NotFound)
    }
}

fn phase(e: &Effect, p: &Play, clock: u64) -> Phase {
    let begin = p.start_us + e.start_delay_us as u64;
    if clock < begin {
        return Phase::Delay;
    }
    let t = clock - begin;
    if e.duration_us == INFINITE {
        return Phase::Active(t);
    }
    if e.duration_us == 0 {
        return Phase::Finished;
    }
    let d = e.duration_us as u64;
    if p.iterations != INFINITE && t >= d * p.iterations as u64 {
        return Phase::Finished;
    }
    Phase::Active(t % d)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Kind, Routing, Waveform};

    fn constant(mag: i32) -> Effect {
        Effect::new(Kind::Constant { magnitude: mag })
    }

    #[test]
    fn handles_are_unique_and_full_is_reported() {
        let mut e = Engine::new(2);
        let a = e.create(constant(1)).unwrap();
        let b = e.create(constant(1)).unwrap();
        assert_ne!(a, b);
        assert_eq!(e.create(constant(1)), Err(EngineError::DeviceFull));
        e.destroy(a).unwrap();
        assert!(e.create(constant(1)).is_ok());
        // El original devolvía/borraba el último nodo con un handle inválido.
        assert_eq!(e.destroy(999), Err(EngineError::NotFound));
        assert_eq!(e.stop(999), Err(EngineError::NotFound));
    }

    #[test]
    fn effects_are_summed_and_saturated() {
        let mut e = Engine::new(8);
        let a = e.create(constant(6000)).unwrap();
        let b = e.create(constant(-6000)).unwrap(); // el signo no cancela: es vibración
        e.start(a, 1, false, 0).unwrap();
        assert_eq!(e.mix(0), Levels { strong: 0.6, weak: 0.6 });
        e.start(b, 1, false, 0).unwrap();
        assert_eq!(e.mix(0), Levels { strong: 1.0, weak: 1.0 });
        e.stop(a).unwrap(); // detener uno NO corta a los demás (bug del original)
        assert_eq!(e.mix(0), Levels { strong: 0.6, weak: 0.6 });
    }

    #[test]
    fn routing_selects_motor() {
        let mut e = Engine::new(8);
        let mut eff = constant(10000);
        eff.routing = Routing::from_cartesian(1.0, 0.0);
        let h = e.create(eff).unwrap();
        e.start(h, 1, false, 0).unwrap();
        assert_eq!(e.mix(0), Levels { strong: 1.0, weak: 0.0 });
    }

    #[test]
    fn duration_iterations_and_delay() {
        let mut e = Engine::new(8);
        let mut eff = constant(10000);
        eff.duration_us = 1000;
        eff.start_delay_us = 500;
        let h = e.create(eff).unwrap();
        e.start(h, 2, false, 0).unwrap();
        assert!(e.is_playing(h, 100).unwrap()); // en el delay cuenta como playing
        assert_eq!(e.mix(100), Levels::OFF);
        assert_eq!(e.mix(600).strong, 1.0);
        assert_eq!(e.mix(2400).strong, 1.0); // segunda iteración
        assert_eq!(e.mix(2600), Levels::OFF);
        assert!(!e.is_playing(h, 2600).unwrap());
        assert!(e.state(2600).stopped);
    }

    #[test]
    fn short_effects_are_not_lost() {
        // En el original, duraciones < 50 ms nunca sonaban.
        let mut e = Engine::new(8);
        let mut eff = constant(10000);
        eff.duration_us = 20_000;
        let h = e.create(eff).unwrap();
        e.start(h, 1, false, 0).unwrap();
        assert_eq!(e.mix(4_000).strong, 1.0);
    }

    #[test]
    fn update_keeps_playback_running() {
        let mut e = Engine::new(8);
        let mut eff = constant(10000);
        eff.duration_us = 1_000_000;
        let h = e.create(eff.clone()).unwrap();
        e.start(h, 1, false, 0).unwrap();
        eff.kind = Kind::Constant { magnitude: 5000 };
        e.update(h, eff).unwrap();
        assert_eq!(e.mix(900_000).strong, 0.5);
        assert_eq!(e.mix(1_000_001), Levels::OFF); // la duración no se reinició
    }

    #[test]
    fn solo_gain_pause_and_actuators() {
        let mut e = Engine::new(8);
        let a = e.create(constant(4000)).unwrap();
        let b = e.create(constant(4000)).unwrap();
        e.start(a, INFINITE, false, 0).unwrap();
        e.start(b, INFINITE, true, 0).unwrap();
        assert!(!e.is_playing(a, 0).unwrap());
        assert_eq!(e.mix(0).strong, 0.4);

        e.set_gain(5000);
        assert_eq!(e.mix(0).strong, 0.2);

        e.pause(10);
        assert_eq!(e.mix(20), Levels::OFF);
        assert!(e.state(20).paused);
        e.resume(30);
        assert_eq!(e.mix(40).strong, 0.2);

        e.set_actuators(false);
        assert_eq!(e.mix(50), Levels::OFF);
    }

    #[test]
    fn pause_freezes_effect_time() {
        let mut e = Engine::new(8);
        let mut eff = constant(10000);
        eff.duration_us = 1000;
        let h = e.create(eff).unwrap();
        e.start(h, 1, false, 0).unwrap();
        e.pause(500);
        e.resume(10_000);
        // 500 µs antes de la pausa + 400 después: todavía dentro de los 1000 µs.
        assert_eq!(e.mix(10_400).strong, 1.0);
        assert_eq!(e.mix(10_600), Levels::OFF);
    }

    #[test]
    fn reset_clears_everything() {
        let mut e = Engine::new(8);
        let h = e.create(constant(1)).unwrap();
        e.pause(0);
        e.reset();
        assert!(e.state(0).empty && !e.state(0).paused);
        assert_eq!(e.effect(h).err(), Some(EngineError::NotFound));
    }

    #[test]
    fn periodic_pulses_between_zero_and_max() {
        let mut e = Engine::new(8);
        let h = e
            .create(Effect::new(Kind::Periodic {
                waveform: Waveform::Square,
                magnitude: 10000,
                offset: 0,
                phase: 0,
                period_us: 100_000,
            }))
            .unwrap();
        e.start(h, INFINITE, false, 0).unwrap();
        assert_eq!(e.mix(10_000).strong, 1.0);
        assert_eq!(e.mix(60_000).strong, 1.0); // |−1|
    }
}

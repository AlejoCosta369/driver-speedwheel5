//! `DIEFFECT` (+ parámetros específicos del tipo) → [`sw5_core::Effect`].

use std::mem::offset_of;

use sw5_core::{ConditionKind, Effect, Envelope, Kind, Routing, Waveform};
use sw5_win::oem::*;
use windows::Win32::Devices::HumanInterfaceDevice::{
    DICONSTANTFORCE, DICUSTOMFORCE, DIEFF_CARTESIAN, DIEFF_POLAR, DIEFF_SPHERICAL, DIEFFECT, DIENVELOPE,
    DIERR_INVALIDPARAM, DIPERIODIC, DIRAMPFORCE,
};
use windows_core::HRESULT;

const INVALID: HRESULT = HRESULT(DIERR_INVALIDPARAM);

/// Lee los parámetros específicos del tipo si el buffer alcanza.
///
/// # Safety
/// `p.lpvTypeSpecificParams` debe apuntar a `p.cbTypeSpecificParams` bytes válidos.
unsafe fn type_specific<T: Copy>(p: &DIEFFECT) -> Result<T, HRESULT> {
    if p.lpvTypeSpecificParams.is_null() || (p.cbTypeSpecificParams as usize) < size_of::<T>() {
        return Err(INVALID);
    }
    Ok(unsafe { p.lpvTypeSpecificParams.cast::<T>().read_unaligned() })
}

fn routing(p: &DIEFFECT) -> Routing {
    if p.cAxes < 2 || p.rglDirection.is_null() {
        // Un solo eje (lo normal en un volante): la fuerza va a los dos motores.
        return Routing::BOTH;
    }
    let dir = unsafe { std::slice::from_raw_parts(p.rglDirection, p.cAxes as usize) };
    if p.dwFlags & DIEFF_CARTESIAN != 0 {
        Routing::from_cartesian(dir[0] as f32, dir[1] as f32)
    } else if p.dwFlags & DIEFF_POLAR != 0 {
        Routing::from_polar(dir[0])
    } else if p.dwFlags & DIEFF_SPHERICAL != 0 {
        Routing::from_spherical(dir[0])
    } else {
        Routing::BOTH
    }
}

/// # Safety
/// `p` debe venir de DirectInput: todos sus punteros válidos según los contadores.
pub unsafe fn effect_from_di(effect_id: u32, p: &DIEFFECT) -> Result<Effect, HRESULT> {
    // DIEFFECT de DirectX 5 no tiene dwStartDelay.
    let dx5_size = offset_of!(DIEFFECT, dwStartDelay);
    if (p.dwSize as usize) < dx5_size {
        return Err(INVALID);
    }
    let periodic = |waveform| -> Result<Kind, HRESULT> {
        let t: DIPERIODIC = unsafe { type_specific(p)? };
        Ok(Kind::Periodic {
            waveform,
            magnitude: t.dwMagnitude.min(10_000),
            offset: t.lOffset.clamp(-10_000, 10_000),
            phase: t.dwPhase,
            period_us: t.dwPeriod,
        })
    };
    let kind = match effect_id {
        ID_CONSTANT => {
            let t: DICONSTANTFORCE = unsafe { type_specific(p)? };
            Kind::Constant { magnitude: t.lMagnitude.clamp(-10_000, 10_000) }
        }
        ID_RAMP => {
            let t: DIRAMPFORCE = unsafe { type_specific(p)? };
            Kind::Ramp { start: t.lStart.clamp(-10_000, 10_000), end: t.lEnd.clamp(-10_000, 10_000) }
        }
        ID_SQUARE => periodic(Waveform::Square)?,
        ID_SINE => periodic(Waveform::Sine)?,
        ID_TRIANGLE => periodic(Waveform::Triangle)?,
        ID_SAWTOOTH_UP => periodic(Waveform::SawtoothUp)?,
        ID_SAWTOOTH_DOWN => periodic(Waveform::SawtoothDown)?,
        ID_SPRING => Kind::Condition(ConditionKind::Spring),
        ID_DAMPER => Kind::Condition(ConditionKind::Damper),
        ID_INERTIA => Kind::Condition(ConditionKind::Inertia),
        ID_FRICTION => Kind::Condition(ConditionKind::Friction),
        ID_CUSTOM => {
            let t: DICUSTOMFORCE = unsafe { type_specific(p)? };
            if t.rglForceData.is_null() || t.cSamples == 0 {
                return Err(INVALID);
            }
            let samples = unsafe { std::slice::from_raw_parts(t.rglForceData, t.cSamples as usize) };
            Kind::Custom {
                samples: samples.iter().map(|&s| s.clamp(-10_000, 10_000)).collect(),
                channels: t.cChannels.max(1),
                sample_period_us: if t.dwSamplePeriod != 0 { t.dwSamplePeriod } else { p.dwSamplePeriod },
            }
        }
        _ => return Err(INVALID),
    };
    let envelope = (!p.lpEnvelope.is_null()).then(|| {
        let e: DIENVELOPE = unsafe { p.lpEnvelope.read_unaligned() };
        Envelope {
            attack_level: e.dwAttackLevel.min(10_000),
            attack_time_us: e.dwAttackTime,
            fade_level: e.dwFadeLevel.min(10_000),
            fade_time_us: e.dwFadeTime,
        }
    });
    Ok(Effect {
        kind,
        duration_us: p.dwDuration,
        start_delay_us: if p.dwSize as usize >= size_of::<DIEFFECT>() { p.dwStartDelay } else { 0 },
        gain: p.dwGain.min(10_000),
        envelope,
        routing: routing(p),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::ffi::c_void;

    fn base(cb: u32, ts: *mut c_void) -> DIEFFECT {
        DIEFFECT {
            dwSize: size_of::<DIEFFECT>() as u32,
            dwFlags: DIEFF_CARTESIAN,
            dwDuration: 500_000,
            dwGain: 10_000,
            cbTypeSpecificParams: cb,
            lpvTypeSpecificParams: ts,
            dwStartDelay: 1000,
            ..Default::default()
        }
    }

    #[test]
    fn constant_one_axis_goes_to_both_motors() {
        let mut c = DICONSTANTFORCE { lMagnitude: -7000 };
        let mut axes = [0u32];
        let mut dir = [1i32];
        let mut p = base(size_of::<DICONSTANTFORCE>() as u32, (&raw mut c).cast());
        p.cAxes = 1;
        p.rgdwAxes = axes.as_mut_ptr();
        p.rglDirection = dir.as_mut_ptr();
        let e = unsafe { effect_from_di(ID_CONSTANT, &p) }.unwrap();
        assert_eq!(e.kind, Kind::Constant { magnitude: -7000 });
        assert_eq!(e.routing, Routing::BOTH);
        assert_eq!((e.duration_us, e.start_delay_us), (500_000, 1000));
    }

    #[test]
    fn two_axis_direction_routes_motors() {
        let mut t = DIPERIODIC { dwMagnitude: 20_000, lOffset: 0, dwPhase: 0, dwPeriod: 100_000 };
        let mut dir = [0i32, 10_000];
        let mut p = base(size_of::<DIPERIODIC>() as u32, (&raw mut t).cast());
        p.cAxes = 2;
        p.rglDirection = dir.as_mut_ptr();
        let e = unsafe { effect_from_di(ID_SINE, &p) }.unwrap();
        assert_eq!(e.routing, Routing { strong: 0.0, weak: 1.0 });
        // Magnitud recortada al nominal.
        assert!(matches!(e.kind, Kind::Periodic { magnitude: 10_000, waveform: Waveform::Sine, .. }));
    }

    #[test]
    fn dx5_struct_has_no_start_delay_and_bad_params_are_rejected() {
        let mut c = DICONSTANTFORCE { lMagnitude: 1 };
        let mut p = base(size_of::<DICONSTANTFORCE>() as u32, (&raw mut c).cast());
        p.dwSize = offset_of!(DIEFFECT, dwStartDelay) as u32;
        assert_eq!(unsafe { effect_from_di(ID_CONSTANT, &p) }.unwrap().start_delay_us, 0);
        p.cbTypeSpecificParams = 0;
        assert_eq!(unsafe { effect_from_di(ID_CONSTANT, &p) }, Err(INVALID));
        assert_eq!(unsafe { effect_from_di(0x999, &p) }, Err(INVALID));
    }
}

//! `sw5-linux test`: sube efectos al volante virtual por la API de force feedback
//! de evdev, igual que lo haría SDL o un juego, y los reproduce.

use std::time::Duration;

use anyhow::{Context, Result, bail};
use evdev::{Device, FFEffectData, FFEffectKind, FFEnvelope, FFReplay, FFTrigger, FFWaveform};

use crate::bridge;

fn data(kind: FFEffectKind, ms: u16) -> FFEffectData {
    FFEffectData {
        direction: 0x4000,
        trigger: FFTrigger { button: 0, interval: 0 },
        replay: FFReplay { length: ms, delay: 0 },
        kind,
    }
}

pub fn run(ms: u16) -> Result<()> {
    let Some(path) = bridge::find_virtual().into_iter().next() else {
        bail!("no está el volante virtual: ¿está corriendo el servicio? (sw5-linux status)");
    };
    let mut dev = Device::open(&path).with_context(|| format!("no se pudo abrir {}", path.display()))?;
    println!("volante virtual: {}", path.display());
    let none = FFEnvelope { attack_length: 0, attack_level: 0, fade_length: 0, fade_level: 0 };
    let tests = [
        ("rumble: los dos motores", FFEffectKind::Rumble { strong_magnitude: 0xFFFF, weak_magnitude: 0xFFFF }),
        ("rumble: solo el motor fuerte", FFEffectKind::Rumble { strong_magnitude: 0xFFFF, weak_magnitude: 0 }),
        ("rumble: solo el motor suave", FFEffectKind::Rumble { strong_magnitude: 0, weak_magnitude: 0xFFFF }),
        (
            "seno de 250 ms: pulsos",
            FFEffectKind::Periodic {
                waveform: FFWaveform::Sine,
                period: 250,
                magnitude: 0x7FFF,
                offset: 0,
                phase: 0,
                envelope: none,
            },
        ),
        (
            "constante con subida de 1 s",
            FFEffectKind::Constant {
                level: 0x7FFF,
                envelope: FFEnvelope { attack_length: 1000, attack_level: 0, fade_length: 0, fade_level: 0 },
            },
        ),
    ];
    dev.set_ff_gain(0xFFFF)?;
    for (name, kind) in tests {
        println!("▶ {name} ({ms} ms)");
        let mut effect = dev.upload_ff_effect(data(kind, ms)).with_context(|| format!("upload de \"{name}\""))?;
        effect.play(1)?;
        std::thread::sleep(Duration::from_millis(ms as u64 + 400));
        drop(effect); // borra el efecto
    }
    println!("prueba terminada");
    Ok(())
}

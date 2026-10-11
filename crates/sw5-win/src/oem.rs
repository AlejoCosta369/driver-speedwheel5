//! Layout de las claves OEM de DirectInput para el volante y tabla de efectos.
//!
//! DirectInput lee `HKLM\...\Joystick\OEM\VID_1A5C&PID_xxxx` para saber qué effect
//! driver cargar (`OEMForceFeedback\CLSID`) y qué efectos anunciar
//! (`OEMForceFeedback\Effects\{GUID}\Attributes` = `DIEFFECTATTRIBUTES`). El
//! `dwEffectId` de esa tabla es lo que el driver recibe en `DownloadEffect`.

use windows::core::Result;

use crate::reg::Key;

pub const OEM_ROOT: &str = r"SYSTEM\CurrentControlSet\Control\MediaProperties\PrivateProperties\Joystick\OEM";

pub fn oem_key_name(pid: u16) -> String {
    format!("VID_{:04X}&PID_{:04X}", crate::VID, pid)
}

pub const ID_CONSTANT: u32 = 0x101;
pub const ID_RAMP: u32 = 0x102;
pub const ID_SQUARE: u32 = 0x103;
pub const ID_SINE: u32 = 0x104;
pub const ID_TRIANGLE: u32 = 0x105;
pub const ID_SAWTOOTH_UP: u32 = 0x106;
pub const ID_SAWTOOTH_DOWN: u32 = 0x107;
pub const ID_SPRING: u32 = 0x108;
pub const ID_DAMPER: u32 = 0x109;
pub const ID_INERTIA: u32 = 0x10A;
pub const ID_FRICTION: u32 = 0x10B;
pub const ID_CUSTOM: u32 = 0x10C;

// DIEFT_* / DIEP_* / DIEFF_* (dinput.h)
const DIEFT_CONSTANTFORCE: u32 = 0x01;
const DIEFT_RAMPFORCE: u32 = 0x02;
const DIEFT_PERIODIC: u32 = 0x03;
const DIEFT_CONDITION: u32 = 0x04;
const DIEFT_CUSTOMFORCE: u32 = 0x05;
const DIEFT_FFATTACK: u32 = 0x200;
const DIEFT_FFFADE: u32 = 0x400;
const DIEFT_SATURATION: u32 = 0x800;
const DIEFT_POSNEGCOEFFICIENTS: u32 = 0x1000;
const DIEFT_DEADBAND: u32 = 0x4000;
const DIEFT_STARTDELAY: u32 = 0x8000;
const DIEP_AXES: u32 = 0x20;
const DIEFF_CARTESIAN_POLAR: u32 = 0x10 | 0x20;

// Parámetros estáticos tal como los declaraba el instalador original.
const STATIC_CONSTANT: u32 = 0x3ED;
const STATIC_FORCE: u32 = 0x3EF;
const STATIC_CONDITION: u32 = 0x36D;

const FORCE_TYPE: u32 = DIEFT_FFATTACK | DIEFT_FFFADE | DIEFT_STARTDELAY;
const CONDITION_TYPE: u32 =
    DIEFT_CONDITION | DIEFT_SATURATION | DIEFT_POSNEGCOEFFICIENTS | DIEFT_DEADBAND | DIEFT_STARTDELAY;

pub struct EffectSpec {
    pub guid: &'static str,
    pub name: &'static str,
    pub id: u32,
    pub eff_type: u32,
    pub static_params: u32,
}

impl EffectSpec {
    /// Todo lo estático salvo los ejes se puede cambiar con el efecto en marcha.
    /// El original declaraba 0, lo que obligaba a DirectInput a reiniciar el
    /// efecto en cada actualización (tirones).
    pub fn dynamic_params(&self) -> u32 {
        self.static_params & !DIEP_AXES
    }

    /// `DIEFFECTATTRIBUTES` serializado (20 bytes).
    pub fn attributes(&self) -> [u8; 20] {
        let mut b = [0u8; 20];
        for (i, v) in [self.id, self.eff_type, self.static_params, self.dynamic_params(), DIEFF_CARTESIAN_POLAR]
            .into_iter()
            .enumerate()
        {
            b[i * 4..i * 4 + 4].copy_from_slice(&v.to_le_bytes());
        }
        b
    }
}

pub const EFFECTS: [EffectSpec; 12] = [
    EffectSpec {
        guid: "{13541C20-8E33-11D0-9AD0-00A0C9A06E35}",
        name: "ConstantForce",
        id: ID_CONSTANT,
        eff_type: DIEFT_CONSTANTFORCE | FORCE_TYPE,
        static_params: STATIC_CONSTANT,
    },
    EffectSpec {
        guid: "{13541C21-8E33-11D0-9AD0-00A0C9A06E35}",
        name: "RampForce",
        id: ID_RAMP,
        eff_type: DIEFT_RAMPFORCE | FORCE_TYPE,
        static_params: STATIC_FORCE,
    },
    EffectSpec {
        guid: "{13541C22-8E33-11D0-9AD0-00A0C9A06E35}",
        name: "Square",
        id: ID_SQUARE,
        eff_type: DIEFT_PERIODIC | FORCE_TYPE,
        static_params: STATIC_FORCE,
    },
    EffectSpec {
        guid: "{13541C23-8E33-11D0-9AD0-00A0C9A06E35}",
        name: "Sine",
        id: ID_SINE,
        eff_type: DIEFT_PERIODIC | FORCE_TYPE,
        static_params: STATIC_FORCE,
    },
    EffectSpec {
        guid: "{13541C24-8E33-11D0-9AD0-00A0C9A06E35}",
        name: "Triangle",
        id: ID_TRIANGLE,
        eff_type: DIEFT_PERIODIC | FORCE_TYPE,
        static_params: STATIC_FORCE,
    },
    EffectSpec {
        guid: "{13541C25-8E33-11D0-9AD0-00A0C9A06E35}",
        name: "SawtoothUp",
        id: ID_SAWTOOTH_UP,
        eff_type: DIEFT_PERIODIC | FORCE_TYPE,
        static_params: STATIC_FORCE,
    },
    EffectSpec {
        guid: "{13541C26-8E33-11D0-9AD0-00A0C9A06E35}",
        name: "SawtoothDown",
        id: ID_SAWTOOTH_DOWN,
        eff_type: DIEFT_PERIODIC | FORCE_TYPE,
        static_params: STATIC_FORCE,
    },
    EffectSpec {
        guid: "{13541C27-8E33-11D0-9AD0-00A0C9A06E35}",
        name: "Spring",
        id: ID_SPRING,
        eff_type: CONDITION_TYPE,
        static_params: STATIC_CONDITION,
    },
    EffectSpec {
        guid: "{13541C28-8E33-11D0-9AD0-00A0C9A06E35}",
        name: "Damper",
        id: ID_DAMPER,
        eff_type: CONDITION_TYPE,
        static_params: STATIC_CONDITION,
    },
    EffectSpec {
        guid: "{13541C29-8E33-11D0-9AD0-00A0C9A06E35}",
        name: "Inertia",
        id: ID_INERTIA,
        eff_type: CONDITION_TYPE,
        static_params: STATIC_CONDITION,
    },
    EffectSpec {
        guid: "{13541C2A-8E33-11D0-9AD0-00A0C9A06E35}",
        name: "Friction",
        id: ID_FRICTION,
        eff_type: CONDITION_TYPE,
        static_params: STATIC_CONDITION,
    },
    EffectSpec {
        guid: "{13541C2B-8E33-11D0-9AD0-00A0C9A06E35}",
        name: "CustomForce",
        id: ID_CUSTOM,
        eff_type: DIEFT_CUSTOMFORCE | FORCE_TYPE,
        static_params: STATIC_FORCE,
    },
];

/// `DIFFDEVICEATTRIBUTES`: flags 0, período de muestreo y resolución mínima de 1 ms.
const FF_DEVICE_ATTRIBUTES: [u8; 12] = [0, 0, 0, 0, 0xE8, 0x03, 0, 0, 0xE8, 0x03, 0, 0];

/// Escribe `OEMForceFeedback` (CLSID, atributos y tabla de efectos) bajo la clave
/// OEM del volante, reemplazando la del driver anterior.
pub fn write_force_feedback(oem: &Key, clsid: &str) -> Result<()> {
    oem.delete_tree("OEMForceFeedback")?;
    let ff = oem.subkey("OEMForceFeedback")?;
    ff.set_sz("CLSID", clsid)?;
    ff.set_binary("Attributes", &FF_DEVICE_ATTRIBUTES)?;
    let effects = ff.subkey("Effects")?;
    for e in &EFFECTS {
        let k = effects.subkey(e.guid)?;
        k.set_sz("", e.name)?;
        k.set_binary("Attributes", &e.attributes())?;
    }
    Ok(())
}

/// Layout completo para una máquina donde el driver original nunca se instaló.
/// Replica los valores del instalador original salvo los que nadie lee
/// (`Amplify`, `Delay`) y la página de propiedades vieja (`ConfigCLSID`).
pub fn write_base_layout(oem: &Key) -> Result<()> {
    oem.set_sz("OEMName", "Speed Wheel 5 Pro")?;
    oem.set_sz("OEMCallout", "joyhid.vxd")?;
    oem.set_binary("OEMData", &[0x03, 0x00, 0x08, 0x10, 0x0C, 0x00, 0x00, 0x00])?;
    oem.set_binary("Flags1", &[0x04, 0x00, 0x00, 0x00])?;
    let axes = oem.subkey("Axes")?;
    for (i, name, usage) in [(0, "X axis", 0x30u16), (1, "Y axis", 0x31), (2, "Z axis", 0x32), (5, "Rz axis", 0x35)] {
        let k = axes.subkey(&i.to_string())?;
        k.set_sz("", name)?;
        // DIOBJECTATTRIBUTES: FFACTUATOR|ASPECTPOSITION|POLLED, usage page 1.
        let mut a = [0x01, 0x81, 0x00, 0x00, 0x01, 0x00, 0, 0];
        a[6..8].copy_from_slice(&usage.to_le_bytes());
        k.set_binary("Attributes", &a)?;
        // DIOBJECTFFATTRIBUTES: fuerza máx. 10, resolución 256.
        k.set_binary("FFAttributes", &[0x0A, 0, 0, 0, 0x00, 0x01, 0, 0])?;
    }
    let buttons = oem.subkey("Buttons")?;
    for i in 0..12u16 {
        let k = buttons.subkey(&i.to_string())?;
        let mut a = [0x02, 0x80, 0x00, 0x00, 0x09, 0x00, 0, 0];
        a[6..8].copy_from_slice(&(i + 1).to_le_bytes());
        k.set_binary("Attributes", &a)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attributes_match_original_layout_except_dynamic_params() {
        // Valor del instalador original para ConstantForce:
        // 0101000001860000ed0300000000000030000000 (dynamic = 0).
        let a = EFFECTS[0].attributes();
        assert_eq!(&a[..12], &[0x01, 0x01, 0, 0, 0x01, 0x86, 0, 0, 0xED, 0x03, 0, 0]);
        assert_eq!(u32::from_le_bytes(a[12..16].try_into().unwrap()), 0x3CD);
        assert_eq!(&a[16..], &[0x30, 0, 0, 0]);
        // Spring: 0801000004d800006d03...
        assert_eq!(&EFFECTS[7].attributes()[..12], &[0x08, 0x01, 0, 0, 0x04, 0xD8, 0, 0, 0x6D, 0x03, 0, 0]);
    }
}

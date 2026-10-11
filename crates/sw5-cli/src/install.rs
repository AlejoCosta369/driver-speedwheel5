//! Instalación, desinstalación, estado y configuración del driver.
//!
//! Qué toca `install`:
//! 1. Copia `sw5.exe` y las DLL x64/x86 a `%ProgramFiles%\SpeedWheel5`.
//! 2. Registra el CLSID nuevo en las dos vistas del registro (64 y 32 bits).
//! 3. Antes de modificar nada, exporta las claves OEM del volante a
//!    `%ProgramData%\SpeedWheel5\backup` (solo la primera vez: ese backup es el
//!    estado original y es lo que `uninstall` restaura).
//! 4. Reescribe `OEMForceFeedback` de cada PID para que DirectInput cargue el
//!    driver nuevo. El driver original queda en el disco, intacto.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use sw5_win::oem::{self, OEM_ROOT, oem_key_name};
use sw5_win::reg::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, Key, View, delete_tree};
use sw5_win::{CLSID_STR, CONFIG_KEY, PIDS};
use windows::Win32::Storage::FileSystem::{MOVEFILE_DELAY_UNTIL_REBOOT, MoveFileExW};
use windows::Win32::System::Registry::HKEY;
use windows_core::{HSTRING, PCWSTR};

use crate::out::say;

const CLSID_PARENT: &str = r"SOFTWARE\Classes\CLSID";
const UNINSTALL_KEY: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\SpeedWheel5Rust";
const ORIGINAL_CLSID: &str = "{2940F080-75AE-11d7-BC91-00501a5c3012}";
const DLL: &str = "sw5_effectdrv.dll";

fn env_dir(var: &str, default: &str) -> PathBuf {
    PathBuf::from(std::env::var(var).unwrap_or_else(|_| default.into()))
}

pub fn install_dir() -> PathBuf {
    env_dir("ProgramFiles", r"C:\Program Files").join("SpeedWheel5")
}

fn backup_dir() -> PathBuf {
    env_dir("ProgramData", r"C:\ProgramData").join("SpeedWheel5").join("backup")
}

fn oem_path(pid: u16) -> String {
    format!(r"{OEM_ROOT}\{}", oem_key_name(pid))
}

/// DirectInput busca el tipo OEM primero en HKCU y después en HKLM. El instalador
/// original escribió las dos copias, así que hay que tratar ambas.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Hive {
    Machine,
    User,
}

impl Hive {
    const ALL: [Hive; 2] = [Hive::Machine, Hive::User];

    fn root(self) -> HKEY {
        match self {
            Hive::Machine => HKEY_LOCAL_MACHINE,
            Hive::User => HKEY_CURRENT_USER,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Hive::Machine => "HKLM",
            Hive::User => "HKCU",
        }
    }

    fn file_stem(self, pid: u16) -> String {
        let k = oem_key_name(pid).replace('&', "_");
        match self {
            Hive::Machine => k, // nombre usado por la primera versión del instalador
            Hive::User => format!("HKCU_{k}"),
        }
    }
}

fn backup_file(hive: Hive, pid: u16) -> PathBuf {
    backup_dir().join(format!("{}.reg", hive.file_stem(pid)))
}

fn absent_marker(hive: Hive, pid: u16) -> PathBuf {
    backup_dir().join(format!("{}.absent", hive.file_stem(pid)))
}

fn reg_exe(args: &[&str]) -> Result<()> {
    let out = Command::new("reg").args(args).arg("/reg:64").output().context("no se pudo ejecutar reg.exe")?;
    if !out.status.success() {
        bail!("reg {}: {}", args[0], String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(())
}

/// Borra un archivo; si está en uso (DLL cargada por un juego), lo programa para el reinicio.
fn remove_or_schedule(p: &Path) {
    if std::fs::remove_file(p).is_ok() || !p.exists() {
        return;
    }
    let h = HSTRING::from(p.as_os_str());
    if unsafe { MoveFileExW(&h, PCWSTR::null(), MOVEFILE_DELAY_UNTIL_REBOOT) }.is_ok() {
        say!("  {} está en uso: se borrará al reiniciar", p.display());
    }
}

/// Copia `src` sobre `dst` aunque `dst` esté cargado por algún proceso
/// (un archivo en uso se puede renombrar pero no sobrescribir).
fn replace_file(src: &Path, dst: &Path) -> Result<()> {
    if dst.exists() && std::fs::canonicalize(src)? == std::fs::canonicalize(dst)? {
        return Ok(());
    }
    if std::fs::copy(src, dst).is_ok() {
        return Ok(());
    }
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_secs();
    let old = dst.with_extension(format!("old-{stamp}"));
    std::fs::rename(dst, &old).with_context(|| format!("no se pudo reemplazar {}", dst.display()))?;
    remove_or_schedule(&old);
    std::fs::copy(src, dst).with_context(|| format!("no se pudo copiar a {}", dst.display()))?;
    Ok(())
}

fn backup_oem_keys() -> Result<()> {
    std::fs::create_dir_all(backup_dir())?;
    for hive in Hive::ALL {
        for pid in PIDS {
            let (file, marker) = (backup_file(hive, pid), absent_marker(hive, pid));
            let label = format!(r"{}\{}", hive.name(), oem_key_name(pid));
            if file.exists() || marker.exists() {
                say!("  backup de {label} ya existe (se conserva el original)");
                continue;
            }
            if Key::open(hive.root(), &oem_path(pid), View::Bits64, false)?.is_some() {
                reg_exe(&["export", &format!(r"{}\{}", hive.name(), oem_path(pid)), &file.to_string_lossy(), "/y"])?;
                say!("  backup: {}", file.display());
            } else {
                std::fs::write(&marker, b"la clave no existia antes de instalar\r\n")?;
                say!("  {label} no existía");
            }
        }
    }
    Ok(())
}

fn register_clsid(view: View, dll: &Path) -> Result<()> {
    let k = Key::create(HKEY_LOCAL_MACHINE, &format!(r"{CLSID_PARENT}\{CLSID_STR}"), view)?;
    k.set_sz("", "Speed Wheel 5 Pro force feedback driver (Rust)")?;
    let s = k.subkey("InprocServer32")?;
    s.set_sz("", &dll.to_string_lossy())?;
    s.set_sz("ThreadingModel", "Both")?;
    Ok(())
}

pub fn install(from: Option<PathBuf>) -> Result<()> {
    let src = match from {
        Some(d) => d,
        None => std::env::current_exe()?.parent().context("ruta del ejecutable")?.to_path_buf(),
    };
    let (src64, src32) = (src.join("x64").join(DLL), src.join("x86").join(DLL));
    for f in [&src64, &src32] {
        if !f.exists() {
            bail!("falta {} (generá la distribución con `cargo xtask dist`)", f.display());
        }
    }

    say!("1/4 backup de las claves del driver original");
    backup_oem_keys()?;

    let dir = install_dir();
    say!("2/4 copiando archivos a {}", dir.display());
    for sub in ["x64", "x86"] {
        std::fs::create_dir_all(dir.join(sub))?;
    }
    let (dst64, dst32) = (dir.join("x64").join(DLL), dir.join("x86").join(DLL));
    replace_file(&src64, &dst64)?;
    replace_file(&src32, &dst32)?;
    let exe = dir.join("sw5.exe");
    replace_file(&std::env::current_exe()?, &exe)?;

    say!("3/4 registrando {CLSID_STR} (64 y 32 bits)");
    register_clsid(View::Bits64, &dst64)?;
    register_clsid(View::Bits32, &dst32)?;

    say!("4/4 apuntando DirectInput al driver nuevo");
    for pid in PIDS {
        let path = oem_path(pid);
        let existed = Key::open(HKEY_LOCAL_MACHINE, &path, View::Bits64, false)?.is_some();
        let k = Key::create(HKEY_LOCAL_MACHINE, &path, View::Bits64)?;
        if !existed {
            oem::write_base_layout(&k)?;
        }
        oem::write_force_feedback(&k, CLSID_STR)?;
        say!(r"  HKLM\{} → {CLSID_STR}", oem_key_name(pid));
        // La copia por usuario tiene prioridad; solo se actualiza si existe.
        if let Some(k) = Key::open(HKEY_CURRENT_USER, &path, View::Bits64, true)? {
            oem::write_force_feedback(&k, CLSID_STR)?;
            say!(r"  HKCU\{} → {CLSID_STR}", oem_key_name(pid));
        }
    }

    let cfg = Key::create(HKEY_LOCAL_MACHINE, CONFIG_KEY, View::Bits64)?;
    cfg.set_sz("InstallDir", &dir.to_string_lossy())?;
    let u = Key::create(HKEY_LOCAL_MACHINE, UNINSTALL_KEY, View::Bits64)?;
    u.set_sz("DisplayName", "Speed Wheel 5 Pro - driver force feedback (Rust)")?;
    u.set_sz("DisplayVersion", env!("CARGO_PKG_VERSION"))?;
    u.set_sz("Publisher", "speedwheel5-rs")?;
    u.set_sz("InstallLocation", &dir.to_string_lossy())?;
    u.set_sz("UninstallString", &format!("\"{}\" uninstall", exe.display()))?;
    u.set_dword("NoModify", 1)?;
    u.set_dword("NoRepair", 1)?;

    say!("listo. Probalo con: sw5 ffb-test");
    Ok(())
}

pub fn uninstall() -> Result<()> {
    say!("1/3 restaurando las claves OEM originales");
    for hive in Hive::ALL {
        for pid in PIDS {
            let (file, marker) = (backup_file(hive, pid), absent_marker(hive, pid));
            let label = format!(r"{}\{}", hive.name(), oem_key_name(pid));
            if file.exists() {
                delete_tree(hive.root(), OEM_ROOT, &oem_key_name(pid), View::Bits64)?;
                reg_exe(&["import", &file.to_string_lossy()])?;
                say!("  {label} restaurada desde {}", file.display());
            } else if marker.exists() {
                // Solo la de HKLM la crea el instalador.
                if hive == Hive::Machine {
                    delete_tree(hive.root(), OEM_ROOT, &oem_key_name(pid), View::Bits64)?;
                    say!("  {label} borrada (no existía antes)");
                }
            } else {
                say!("  sin backup para {label}: se deja como está");
            }
        }
    }

    say!("2/3 borrando el registro del driver");
    for view in [View::Bits64, View::Bits32] {
        delete_tree(HKEY_LOCAL_MACHINE, CLSID_PARENT, CLSID_STR, view)?;
    }
    delete_tree(HKEY_LOCAL_MACHINE, "SOFTWARE", "SpeedWheel5", View::Bits64)?;
    delete_tree(HKEY_CURRENT_USER, "SOFTWARE", "SpeedWheel5", View::Bits64)?;
    delete_tree(
        HKEY_LOCAL_MACHINE,
        r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall",
        "SpeedWheel5Rust",
        View::Bits64,
    )?;

    say!("3/3 borrando archivos");
    let dir = install_dir();
    for f in [dir.join("x64").join(DLL), dir.join("x86").join(DLL), dir.join("sw5.exe")] {
        remove_or_schedule(&f);
    }
    for sub in [dir.join("x64"), dir.join("x86"), dir.clone()] {
        let _ = std::fs::remove_dir(sub);
    }
    let _ = std::fs::remove_dir_all(backup_dir());
    let _ = std::fs::remove_dir(backup_dir().parent().unwrap());
    say!("listo: DirectInput vuelve a usar el driver anterior");
    Ok(())
}

fn describe_clsid(clsid: &str) -> &'static str {
    if clsid.eq_ignore_ascii_case(CLSID_STR) {
        "driver Rust (este)"
    } else if clsid.eq_ignore_ascii_case(ORIGINAL_CLSID) {
        "driver original de JessTech"
    } else {
        "otro driver"
    }
}

pub fn status() -> Result<()> {
    let wheels = sw5_win::hid::find_wheels()?;
    say!("volantes conectados: {}", wheels.len());
    for w in &wheels {
        say!("  {:04X}:{:04X} \"{}\"", w.vid, w.pid, w.product);
    }
    for hive in Hive::ALL {
        for pid in PIDS {
            let ff = Key::open(hive.root(), &format!(r"{}\OEMForceFeedback", oem_path(pid)), View::Bits64, false)?;
            let label = format!(r"{}\{}", hive.name(), oem_key_name(pid));
            match ff.and_then(|k| k.get_sz("CLSID").ok().flatten()) {
                Some(c) => say!("{label}: OEMForceFeedback = {c} ({})", describe_clsid(&c)),
                None if hive == Hive::Machine => say!("{label}: sin OEMForceFeedback"),
                None => {}
            }
        }
    }
    for (view, name) in [(View::Bits64, "64 bits"), (View::Bits32, "32 bits")] {
        let k = Key::open(HKEY_LOCAL_MACHINE, &format!(r"{CLSID_PARENT}\{CLSID_STR}\InprocServer32"), view, false)?;
        match k.and_then(|k| k.get_sz("").ok().flatten()) {
            Some(p) => {
                let ok = if Path::new(&p).exists() { "ok" } else { "¡NO EXISTE!" };
                say!("CLSID {name}: {p} [{ok}]")
            }
            None => say!("CLSID {name}: no registrado"),
        }
    }
    say!(
        "backup del original: {}",
        if backup_dir().exists() { backup_dir().display().to_string() } else { "no hay".into() }
    );
    for hive in Hive::ALL {
        if let Some(k) = Key::open(hive.root(), CONFIG_KEY, View::Bits64, false)? {
            for name in ["Strength", "MinLevel", "Gamma", "SwapMotors", "TickMs", "KeepaliveMs", "LogVerbose"] {
                if let Some(v) = k.get_dword(name)? {
                    say!("config ({}) {name} = {v}", hive.name());
                }
            }
            if let Some(v) = k.get_sz("LogFile")? {
                say!("config ({}) LogFile = {v}", hive.name());
            }
        }
    }
    Ok(())
}

#[derive(Debug, Default, clap::Args)]
pub struct ConfigArgs {
    /// Guardar solo para este usuario (HKCU): no pide administrador y tiene prioridad.
    #[arg(long)]
    pub user: bool,
    /// Intensidad global, 0-100 %.
    #[arg(long, value_parser = clap::value_parser!(u32).range(0..=100))]
    pub strength: Option<u32>,
    /// Byte mínimo con el motor encendido (0-255).
    #[arg(long, value_parser = clap::value_parser!(u32).range(0..=255))]
    pub min_level: Option<u32>,
    /// Curva de respuesta × 100 (100 = lineal).
    #[arg(long)]
    pub gamma: Option<u32>,
    /// Intercambiar motor fuerte y débil.
    #[arg(long)]
    pub swap_motors: Option<bool>,
    /// Período de mezcla en ms (1-50).
    #[arg(long, value_parser = clap::value_parser!(u32).range(1..=50))]
    pub tick_ms: Option<u32>,
    /// Reenvío del report mientras vibra, en ms (0 = desactivado).
    #[arg(long)]
    pub keepalive_ms: Option<u32>,
    /// Archivo de log del driver ("" para desactivarlo).
    #[arg(long)]
    pub log_file: Option<String>,
    /// Loguear cada llamada de DirectInput.
    #[arg(long)]
    pub verbose: Option<bool>,
}

impl ConfigArgs {
    /// Sin valores para cambiar (`--user` solo no cuenta).
    pub fn is_empty(&self) -> bool {
        self.strength.is_none()
            && self.min_level.is_none()
            && self.gamma.is_none()
            && self.swap_motors.is_none()
            && self.tick_ms.is_none()
            && self.keepalive_ms.is_none()
            && self.log_file.is_none()
            && self.verbose.is_none()
    }
}

pub fn config(a: &ConfigArgs) -> Result<()> {
    let root = if a.user { HKEY_CURRENT_USER } else { HKEY_LOCAL_MACHINE };
    let k = Key::create(root, CONFIG_KEY, View::Bits64)?;
    let dwords = [
        ("Strength", a.strength),
        ("MinLevel", a.min_level),
        ("Gamma", a.gamma),
        ("SwapMotors", a.swap_motors.map(u32::from)),
        ("TickMs", a.tick_ms),
        ("KeepaliveMs", a.keepalive_ms),
        ("LogVerbose", a.verbose.map(u32::from)),
    ];
    for (name, v) in dwords {
        if let Some(v) = v {
            k.set_dword(name, v)?;
            say!("{name} = {v}");
        }
    }
    if let Some(f) = &a.log_file {
        k.set_sz("LogFile", f)?;
        say!("LogFile = {f}");
    }
    say!("los cambios se aplican la próxima vez que un juego abra el volante");
    Ok(())
}

//! Instalación en Linux: binario, regla de udev, módulo uinput y servicio systemd.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};

use crate::bridge;

const UDEV_RULE: &str = "/etc/udev/rules.d/71-sw5-wheel.rules";
const MODULES_LOAD: &str = "/etc/modules-load.d/sw5-uinput.conf";
const UNIT: &str = "/etc/systemd/system/sw5-linux.service";

/// Corre en 71-: después de 70-uaccess (que da acceso a los joysticks) y antes de
/// 73-seat-late (que lo aplica), así el `TAG-=` llega a tiempo. Solo matchea el
/// volante USB real: el virtual (uinput) no tiene `idVendor` USB.
const RULE: &str = r#"# sw5-linux: force feedback para el Genius Speed Wheel 5 Pro.
# El servicio expone un volante virtual con force feedback; el dispositivo real se
# oculta a los usuarios para que los juegos usen solo el virtual.
SUBSYSTEM=="input", KERNEL=="event*|js*", ATTRS{idVendor}=="1a5c", ATTRS{idProduct}=="3112|3012", MODE="0600", ENV{ID_INPUT_JOYSTICK}="", TAG-="uaccess"
SUBSYSTEM=="hidraw", ATTRS{idVendor}=="1a5c", ATTRS{idProduct}=="3112|3012", MODE="0600"
"#;

fn require_root() -> Result<()> {
    if unsafe { libc::geteuid() } != 0 {
        bail!("hace falta root: ejecutá con sudo");
    }
    Ok(())
}

fn run(cmd: &str, args: &[&str]) -> Result<()> {
    let st = Command::new(cmd).args(args).status().with_context(|| format!("no se pudo ejecutar {cmd}"))?;
    if !st.success() {
        bail!("{cmd} {} terminó con {st}", args.join(" "));
    }
    Ok(())
}

fn unit_file(bin: &Path, run_args: &[String]) -> String {
    format!(
        "[Unit]\n\
         Description=Force feedback para el Speed Wheel 5 Pro (sw5-linux)\n\
         After=systemd-udevd.service\n\
         \n\
         [Service]\n\
         ExecStart={} run {}\n\
         Restart=on-failure\n\
         RestartSec=2\n\
         \n\
         [Install]\n\
         WantedBy=multi-user.target\n",
        bin.display(),
        run_args.join(" ")
    )
}

fn retrigger_udev() {
    let _ = run("udevadm", &["control", "--reload"]);
    let _ = run("udevadm", &["trigger", "--subsystem-match=input", "--subsystem-match=hidraw"]);
}

/// `run_args`: opciones de `sw5-linux run` que el servicio usará.
pub fn install(prefix: &Path, run_args: &[String]) -> Result<()> {
    require_root()?;
    let bin: PathBuf = prefix.join("bin").join("sw5-linux");
    println!("1/4 copiando el binario a {}", bin.display());
    std::fs::create_dir_all(bin.parent().unwrap())?;
    let me = std::env::current_exe()?;
    if std::fs::canonicalize(&me).ok() != std::fs::canonicalize(&bin).ok() {
        // Copiar a un temporal y renombrar: funciona aunque el servicio esté corriendo.
        let tmp = bin.with_extension("new");
        std::fs::copy(&me, &tmp)?;
        std::fs::rename(&tmp, &bin)?;
    }

    println!("2/4 módulo uinput");
    std::fs::write(MODULES_LOAD, "uinput\n")?;
    let _ = run("modprobe", &["uinput"]);

    println!("3/4 regla de udev ({UDEV_RULE})");
    std::fs::write(UDEV_RULE, RULE)?;
    retrigger_udev();

    println!("4/4 servicio systemd ({UNIT})");
    std::fs::write(UNIT, unit_file(&bin, run_args))?;
    run("systemctl", &["daemon-reload"])?;
    run("systemctl", &["enable", "sw5-linux.service"])?;
    run("systemctl", &["restart", "sw5-linux.service"])?;
    println!("listo. Probalo con: sw5-linux test");
    Ok(())
}

pub fn uninstall(prefix: &Path) -> Result<()> {
    require_root()?;
    println!("1/3 deteniendo el servicio");
    let _ = run("systemctl", &["disable", "--now", "sw5-linux.service"]);
    for f in [UNIT, UDEV_RULE, MODULES_LOAD] {
        match std::fs::remove_file(f) {
            Ok(()) => println!("  borrado {f}"),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e).with_context(|| format!("no se pudo borrar {f}")),
        }
    }
    let _ = run("systemctl", &["daemon-reload"]);
    println!("2/3 restaurando el acceso normal al volante");
    retrigger_udev();
    println!("3/3 borrando el binario");
    let bin = prefix.join("bin").join("sw5-linux");
    if bin.exists() {
        std::fs::remove_file(&bin)?;
    }
    println!("listo");
    Ok(())
}

pub fn status() -> Result<()> {
    let wheels = bridge::find_wheels();
    println!("volantes reales: {}", wheels.len());
    for w in &wheels {
        println!("  1A5C:{:04X} \"{}\"  {}  {}", w.pid, w.name, w.event.display(), w.hidraw.display());
    }
    let virt = bridge::find_virtual();
    println!(
        "volante virtual con force feedback: {}",
        if virt.is_empty() { "no".into() } else { format!("{}", virt[0].display()) }
    );
    let active = Command::new("systemctl")
        .args(["is-active", "sw5-linux.service"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|_| "desconocido".into());
    println!("servicio sw5-linux: {active}");
    println!("regla de udev: {}", if Path::new(UDEV_RULE).exists() { "instalada" } else { "no instalada" });
    Ok(())
}

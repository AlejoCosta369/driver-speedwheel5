//! Tareas de build del proyecto.
//!
//! * `cargo xtask dist [--debug]`: compila la CLI (x64) y el effect driver para x64
//!   y x86, y arma `dist/` con el layout que espera `sw5 install`:
//!
//!   ```text
//!   dist/sw5.exe
//!   dist/x64/sw5_effectdrv.dll (+ .pdb)
//!   dist/x86/sw5_effectdrv.dll (+ .pdb)
//!   ```
//!
//! * `cargo xtask package`: `dist` en release + `dist/SpeedWheel5-driver-<v>.zip`
//!   con los scripts de doble clic y el LEEME de `packaging/windows` (sin .pdb).
//! * `cargo xtask package-linux [--target T]`: `sw5-linux` en release (por defecto
//!   estático con musl) + `dist/sw5-linux-<v>-<arch>.tar.gz`. Corre en Linux.

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

const X64: &str = "x86_64-pc-windows-msvc";
const X86: &str = "i686-pc-windows-msvc";
const VERSION: &str = env!("CARGO_PKG_VERSION");

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_path_buf()
}

fn cargo(args: &[&str]) -> Result<(), String> {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    println!("> cargo {}", args.join(" "));
    let ok = Command::new(cargo).args(args).status().map_err(|e| e.to_string())?.success();
    if ok { Ok(()) } else { Err(format!("falló: cargo {}", args.join(" "))) }
}

fn copy(from: &Path, to: &Path) -> Result<(), String> {
    std::fs::create_dir_all(to.parent().unwrap()).map_err(|e| e.to_string())?;
    std::fs::copy(from, to).map_err(|e| format!("{} → {}: {e}", from.display(), to.display()))?;
    Ok(())
}

fn run(cmd: &mut Command) -> Result<(), String> {
    let ok = cmd.status().map_err(|e| e.to_string())?.success();
    if ok { Ok(()) } else { Err(format!("falló: {cmd:?}")) }
}

/// Copia los archivos de `src` a `stage`, reemplazando `{VERSION}` en los `.txt`
/// y normalizando sus finales de línea (CRLF para Windows, LF para Linux).
fn stage_docs(src: &Path, stage: &Path, crlf: bool) -> Result<(), String> {
    for entry in std::fs::read_dir(src).map_err(|e| e.to_string())? {
        let p = entry.map_err(|e| e.to_string())?.path();
        let file = p.file_name().unwrap().to_owned();
        if p.extension().is_some_and(|e| e == "txt") {
            let text = std::fs::read_to_string(&p).map_err(|e| e.to_string())?;
            let text = text.replace("{VERSION}", VERSION).replace("\r\n", "\n");
            let text = if crlf { text.replace('\n', "\r\n") } else { text };
            std::fs::write(stage.join(&file), text).map_err(|e| e.to_string())?;
        } else {
            copy(&p, &stage.join(&file))?;
        }
    }
    Ok(())
}

fn dist(release: bool) -> Result<(), String> {
    let profile = if release { "release" } else { "debug" };
    let mut base = vec!["build"];
    if release {
        base.push("--release");
    }
    for (pkg, target) in [("sw5-cli", X64), ("sw5-effectdrv", X64), ("sw5-effectdrv", X86)] {
        let mut a = base.clone();
        a.extend(["-p", pkg, "--target", target]);
        cargo(&a)?;
    }
    let t = root().join("target");
    let d = root().join("dist");
    let _ = std::fs::remove_dir_all(&d);
    copy(&t.join(X64).join(profile).join("sw5.exe"), &d.join("sw5.exe"))?;
    for (target, sub) in [(X64, "x64"), (X86, "x86")] {
        for f in ["sw5_effectdrv.dll", "sw5_effectdrv.pdb"] {
            copy(&t.join(target).join(profile).join(f), &d.join(sub).join(f))?;
        }
    }
    println!("distribución lista en {}", d.display());
    Ok(())
}

fn package() -> Result<(), String> {
    dist(true)?;
    let name = format!("SpeedWheel5-driver-{VERSION}");
    let d = root().join("dist");
    let stage = d.join(&name);
    copy(&d.join("sw5.exe"), &stage.join("sw5.exe"))?;
    for sub in ["x64", "x86"] {
        copy(&d.join(sub).join("sw5_effectdrv.dll"), &stage.join(sub).join("sw5_effectdrv.dll"))?;
    }
    stage_docs(&root().join("packaging").join("windows"), &stage, true)?;
    let zip = d.join(format!("{name}.zip"));
    let _ = std::fs::remove_file(&zip);
    run(Command::new("powershell")
        .args(["-NoProfile", "-Command", "Compress-Archive", "-Path"])
        .arg(&stage)
        .arg("-DestinationPath")
        .arg(&zip))?;
    println!("paquete listo: {}", zip.display());
    Ok(())
}

fn package_linux(target: &str) -> Result<(), String> {
    cargo(&["build", "--release", "-p", "sw5-linux", "--target", target])?;
    let arch = target.split('-').next().unwrap_or("x86_64");
    let name = format!("sw5-linux-{VERSION}");
    let d = root().join("dist");
    let stage = d.join(&name);
    let _ = std::fs::remove_dir_all(&stage);
    copy(&root().join("target").join(target).join("release").join("sw5-linux"), &stage.join("sw5-linux"))?;
    stage_docs(&root().join("packaging").join("linux"), &stage, false)?;
    let tgz = d.join(format!("{name}-{arch}.tar.gz"));
    run(Command::new("tar").arg("czf").arg(&tgz).arg("-C").arg(&d).arg(&name))?;
    println!("paquete listo: {}", tgz.display());
    Ok(())
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let flag = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned();
    let result = match args.first().map(String::as_str) {
        Some("dist") => dist(!args.iter().any(|a| a == "--debug")),
        Some("package") => package(),
        Some("package-linux") => package_linux(&flag("--target").unwrap_or_else(|| "x86_64-unknown-linux-musl".into())),
        _ => Err("uso: cargo xtask dist [--debug] | package | package-linux [--target T]".into()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}

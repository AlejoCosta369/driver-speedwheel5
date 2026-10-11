use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use sw5_win::hid::{self, DeviceInfo, FieldCaps, HidDevice};

use crate::out::say;
use crate::{ditest, elevate, install, out, selftest};

#[derive(Parser)]
#[command(name = "sw5", version, about = "Speed Wheel 5 Pro: diagnóstico e instalación del driver force feedback")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
    /// Uso interno: salida del proceso relanzado como administrador.
    #[arg(long, global = true, hide = true)]
    elevated_log: Option<PathBuf>,
}

#[derive(Subcommand)]
enum Cmd {
    /// Lista los volantes conectados con sus capacidades HID.
    List,
    /// Envía un output report crudo [00, 00, flags, a, b] durante `ms` milisegundos.
    Raw {
        /// Motor A, fuerte (byte 3, 0-255).
        a: u8,
        /// Motor B, suave (byte 4, 0-255).
        b: u8,
        #[arg(long, default_value_t = 0)]
        flags: u8,
        #[arg(long, default_value_t = 1000)]
        ms: u64,
        /// Cada cuánto reenviar el report (0 = una sola vez).
        #[arg(long, default_value_t = 50)]
        every: u64,
    },
    /// Muestra los input reports en hexadecimal (solo los que cambian).
    Input {
        #[arg(long, default_value_t = 10)]
        secs: u64,
    },
    /// Instala el driver Rust (pide permisos de administrador).
    Install {
        /// Carpeta con x64\ y x86\ (por defecto, la de este ejecutable).
        #[arg(long)]
        from: Option<PathBuf>,
    },
    /// Desinstala el driver Rust y restaura el original (pide administrador).
    Uninstall,
    /// Muestra qué driver usa DirectInput para el volante y la configuración.
    Status,
    /// Lee o cambia la configuración del driver (cambiarla pide administrador).
    Config(install::ConfigArgs),
    /// Carga la DLL del driver sin instalarla y le manda efectos directamente.
    Selftest {
        /// DLL x64 a probar (por defecto, x64\sw5_effectdrv.dll junto a este ejecutable).
        dll: Option<PathBuf>,
    },
    /// Prueba el force feedback por DirectInput, como lo haría un juego.
    FfbTest {
        #[arg(value_enum, default_value = "all")]
        effect: ditest::TestEffect,
        #[arg(long, default_value_t = 1500)]
        ms: u64,
        /// Magnitud 0-10000.
        #[arg(long, default_value_t = 10_000)]
        magnitude: i32,
    },
}

pub fn main() {
    let cli = Cli::parse();
    if let Some(log) = &cli.elevated_log {
        out::tee_to(log);
    }
    let code = match run(cli) {
        Ok(code) => code,
        Err(e) => {
            say!("error: {e:#}");
            1
        }
    };
    std::process::exit(code as i32);
}

/// Si el comando necesita administrador y no lo somos, se relanza elevado.
fn needs_admin(cmd: &Cmd) -> bool {
    match cmd {
        Cmd::Install { .. } | Cmd::Uninstall => true,
        Cmd::Config(a) => !a.is_empty() && !a.user,
        _ => false,
    }
}

fn run(cli: Cli) -> Result<u32> {
    if needs_admin(&cli.cmd) && !elevate::is_elevated() {
        if cli.elevated_log.is_some() {
            bail!("el proceso relanzado no quedó elevado");
        }
        let args: Vec<String> = std::env::args().skip(1).collect();
        return elevate::relaunch_elevated(&args);
    }
    match cli.cmd {
        Cmd::List => list()?,
        Cmd::Raw { a, b, flags, ms, every } => raw(a, b, flags, ms, every)?,
        Cmd::Input { secs } => input(secs)?,
        Cmd::Install { from } => install::install(from)?,
        Cmd::Uninstall => install::uninstall()?,
        Cmd::Status => install::status()?,
        Cmd::Config(a) => {
            if a.is_empty() {
                install::status()?
            } else {
                install::config(&a)?
            }
        }
        Cmd::Selftest { dll } => {
            let dll = match dll {
                Some(d) => d,
                None => std::env::current_exe()?.with_file_name("x64").join("sw5_effectdrv.dll"),
            };
            selftest::run(&dll)?
        }
        Cmd::FfbTest { effect, ms, magnitude } => ditest::run(effect, ms, magnitude)?,
    }
    Ok(0)
}

fn first_wheel() -> Result<DeviceInfo> {
    hid::find_wheels()?.into_iter().next().context("no se encontró ningún Speed Wheel 5 Pro (VID 1A5C) conectado")
}

fn print_fields(label: &str, fields: &[FieldCaps]) {
    for f in fields {
        say!(
            "    {label:<14} id={} page=0x{:04X} usage=0x{:02X}..0x{:02X} bits={} count={} logical={}..{}",
            f.report_id,
            f.usage_page,
            f.usage_min,
            f.usage_max,
            f.bit_size,
            f.report_count,
            f.logical_min,
            f.logical_max
        );
    }
}

fn list() -> Result<()> {
    let wheels = hid::find_wheels()?;
    if wheels.is_empty() {
        bail!("no hay ningún Speed Wheel 5 Pro conectado");
    }
    for d in &wheels {
        let c = &d.caps;
        say!("{:04X}:{:04X} v{:04X} \"{}\"", d.vid, d.pid, d.version, d.product);
        say!("  ruta: {}", d.path_string());
        say!(
            "  top-level usage page=0x{:04X} usage=0x{:02X}  bytes: input={} output={} feature={}",
            c.usage_page,
            c.usage,
            c.input_len,
            c.output_len,
            c.feature_len
        );
        print_fields("input value", &c.input_values);
        print_fields("input button", &c.input_buttons);
        print_fields("output value", &c.output_values);
        print_fields("output button", &c.output_buttons);
        print_fields("feature value", &c.feature_values);
    }
    Ok(())
}

fn raw(a: u8, b: u8, flags: u8, ms: u64, every: u64) -> Result<()> {
    let info = first_wheel()?;
    let dev = HidDevice::open(&info.path).context("no se pudo abrir el volante para escritura")?;
    let report = [0u8, 0, flags, a, b];
    say!("enviando {report:02X?} durante {ms} ms");
    let start = Instant::now();
    loop {
        dev.write_report(&report, 200).context("WriteFile falló")?;
        if every == 0 || start.elapsed() >= Duration::from_millis(ms) {
            break;
        }
        std::thread::sleep(Duration::from_millis(every.min(ms)));
    }
    if every == 0 {
        std::thread::sleep(Duration::from_millis(ms));
    }
    dev.write_report(&[0u8; 5], 200).context("no se pudo enviar el report de parada")?;
    say!("parada enviada");
    Ok(())
}

fn input(secs: u64) -> Result<()> {
    let info = first_wheel()?;
    let dev = HidDevice::open(&info.path)?;
    let mut buf = vec![0u8; dev.caps.input_len as usize];
    let mut last = Vec::new();
    let end = Instant::now() + Duration::from_secs(secs);
    say!("leyendo input reports de {} bytes durante {secs} s…", buf.len());
    while Instant::now() < end {
        if let Some(n) = dev.read_report(&mut buf, 100)? {
            if buf[..n] != last[..] {
                say!("{:02X?}", &buf[..n]);
                last = buf[..n].to_vec();
            }
        }
    }
    Ok(())
}

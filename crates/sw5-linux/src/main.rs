//! `sw5-linux`: servicio de force feedback para el Speed Wheel 5 Pro en Linux.

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
mod ff;

#[cfg(target_os = "linux")]
mod bridge;
#[cfg(target_os = "linux")]
mod install;
#[cfg(target_os = "linux")]
mod selftest;

#[cfg(target_os = "linux")]
mod app {
    use std::path::PathBuf;

    use anyhow::Result;
    use clap::{Args, Parser, Subcommand};
    use sw5_core::OutputConfig;

    use crate::{bridge, install, selftest};

    #[derive(Parser)]
    #[command(name = "sw5-linux", version, about = "Speed Wheel 5 Pro: force feedback en Linux")]
    struct Cli {
        #[command(subcommand)]
        cmd: Cmd,
    }

    #[derive(Args, Clone)]
    struct RunArgs {
        /// Intensidad global, 0-100 %.
        #[arg(long, default_value_t = 100, value_parser = clap::value_parser!(u32).range(0..=100))]
        strength: u32,
        /// Byte mínimo con el motor encendido (0-255).
        #[arg(long, default_value_t = 0, value_parser = clap::value_parser!(u32).range(0..=255))]
        min_level: u32,
        /// Curva de respuesta × 100 (100 = lineal).
        #[arg(long, default_value_t = 100)]
        gamma: u32,
        /// Intercambiar motor fuerte y suave.
        #[arg(long)]
        swap_motors: bool,
        /// Período de mezcla en ms (1-50).
        #[arg(long, default_value_t = 4, value_parser = clap::value_parser!(u64).range(1..=50))]
        tick_ms: u64,
        /// Reenvío del report mientras vibra, en ms (0 = desactivado).
        #[arg(long, default_value_t = 100)]
        keepalive_ms: u64,
        /// Loguear cada efecto que suben los juegos.
        #[arg(long)]
        verbose: bool,
    }

    impl RunArgs {
        fn options(&self) -> bridge::Options {
            bridge::Options {
                output: OutputConfig {
                    strength: self.strength as f32 / 100.0,
                    min_level: self.min_level as u8,
                    gamma: self.gamma.clamp(10, 1000) as f32 / 100.0,
                    swap_motors: self.swap_motors,
                    ..OutputConfig::default()
                },
                tick_ms: self.tick_ms,
                keepalive_ms: self.keepalive_ms,
                verbose: self.verbose,
            }
        }

        /// Las mismas opciones como argumentos, para la unidad de systemd.
        fn to_args(&self) -> Vec<String> {
            let mut v = vec![
                format!("--strength {}", self.strength),
                format!("--min-level {}", self.min_level),
                format!("--gamma {}", self.gamma),
                format!("--tick-ms {}", self.tick_ms),
                format!("--keepalive-ms {}", self.keepalive_ms),
            ];
            if self.swap_motors {
                v.push("--swap-motors".into());
            }
            if self.verbose {
                v.push("--verbose".into());
            }
            v
        }
    }

    #[derive(Subcommand)]
    enum Cmd {
        /// Corre el servicio en primer plano (lo que ejecuta systemd).
        Run(RunArgs),
        /// Lista los volantes conectados.
        List,
        /// Instala el servicio, la regla de udev y el binario (con sudo).
        Install {
            #[arg(long, default_value = "/usr/local")]
            prefix: PathBuf,
            #[command(flatten)]
            run: RunArgs,
        },
        /// Desinstala todo (con sudo).
        Uninstall {
            #[arg(long, default_value = "/usr/local")]
            prefix: PathBuf,
        },
        /// Estado del servicio y de los volantes.
        Status,
        /// Prueba los efectos en el volante virtual, como un juego.
        Test {
            #[arg(long, default_value_t = 1500)]
            ms: u16,
        },
        /// Report crudo a los motores, sin el servicio (con sudo y el servicio detenido).
        Raw {
            /// Motor fuerte (0-255).
            a: u8,
            /// Motor suave (0-255).
            b: u8,
            #[arg(long, default_value_t = 1000)]
            ms: u64,
        },
    }

    pub fn main() -> Result<()> {
        match Cli::parse().cmd {
            Cmd::Run(a) => bridge::run_forever(&a.options()),
            Cmd::List => {
                let w = bridge::find_wheels();
                if w.is_empty() {
                    println!("no hay ningún Speed Wheel 5 Pro conectado (o falta permiso: probá con sudo)");
                }
                for w in w {
                    println!("1A5C:{:04X} \"{}\"  {}  {}", w.pid, w.name, w.event.display(), w.hidraw.display());
                }
                Ok(())
            }
            Cmd::Install { prefix, run } => install::install(&prefix, &run.to_args()),
            Cmd::Uninstall { prefix } => install::uninstall(&prefix),
            Cmd::Status => install::status(),
            Cmd::Test { ms } => selftest::run(ms),
            Cmd::Raw { a, b, ms } => bridge::raw(a, b, ms),
        }
    }
}

#[cfg(target_os = "linux")]
fn main() -> anyhow::Result<()> {
    app::main()
}

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("sw5-linux es para Linux; en Windows usá sw5.exe");
    std::process::exit(1);
}

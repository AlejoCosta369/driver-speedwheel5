//! `sw5`: herramienta de Windows. En Linux se usa `sw5-linux`.

#[cfg(windows)]
mod app;
#[cfg(windows)]
mod ditest;
#[cfg(windows)]
mod elevate;
#[cfg(windows)]
mod install;
#[cfg(windows)]
mod out;
#[cfg(windows)]
mod selftest;

#[cfg(windows)]
fn main() {
    app::main()
}

#[cfg(not(windows))]
fn main() {
    eprintln!("sw5 es la herramienta de Windows; en Linux usá sw5-linux");
    std::process::exit(1);
}

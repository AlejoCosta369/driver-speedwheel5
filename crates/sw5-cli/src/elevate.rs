//! Detección de privilegios y relanzamiento elevado (UAC).

use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;

use anyhow::{Context, Result};
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::Security::{GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation};
use windows::Win32::System::Threading::{
    GetCurrentProcess, GetExitCodeProcess, INFINITE, OpenProcessToken, WaitForSingleObject,
};
use windows::Win32::UI::Shell::{SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, ShellExecuteExW};
use windows::Win32::UI::WindowsAndMessaging::SW_HIDE;
use windows_core::{PCWSTR, w};

pub fn is_elevated() -> bool {
    unsafe {
        let mut token = HANDLE::default();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).is_err() {
            return false;
        }
        let mut elev = TOKEN_ELEVATION::default();
        let mut len = 0u32;
        let ok = GetTokenInformation(
            token,
            TokenElevation,
            Some((&raw mut elev).cast()),
            size_of::<TOKEN_ELEVATION>() as u32,
            &mut len,
        )
        .is_ok();
        let _ = CloseHandle(token);
        ok && elev.TokenIsElevated != 0
    }
}

fn quote(s: &str) -> String {
    if !s.is_empty() && !s.contains([' ', '\t', '"']) {
        return s.to_string();
    }
    format!("\"{}\"", s.replace('"', "\\\""))
}

fn wide(s: &OsStr) -> Vec<u16> {
    s.encode_wide().chain(Some(0)).collect()
}

/// Relanza este ejecutable como administrador con los mismos argumentos, espera a
/// que termine, muestra su salida y devuelve su código de salida.
pub fn relaunch_elevated(args: &[String]) -> Result<u32> {
    let exe = std::env::current_exe()?;
    let log = std::env::temp_dir().join(format!("sw5-elevated-{}.log", std::process::id()));
    let mut params: Vec<String> = args.iter().map(|a| quote(a)).collect();
    params.push("--elevated-log".into());
    params.push(quote(&log.to_string_lossy()));
    let params = wide(OsStr::new(&params.join(" ")));
    let file = wide(exe.as_os_str());

    println!("se necesitan permisos de administrador: aceptá el aviso de Windows (UAC)…");
    let mut info = SHELLEXECUTEINFOW {
        cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC,
        lpVerb: w!("runas"),
        lpFile: PCWSTR(file.as_ptr()),
        lpParameters: PCWSTR(params.as_ptr()),
        nShow: SW_HIDE.0,
        ..Default::default()
    };
    unsafe { ShellExecuteExW(&mut info) }.context("no se pudo elevar (¿se canceló el aviso UAC?)")?;
    let mut code = 1u32;
    unsafe {
        WaitForSingleObject(info.hProcess, INFINITE);
        let _ = GetExitCodeProcess(info.hProcess, &mut code);
        let _ = CloseHandle(info.hProcess);
    }
    if let Ok(text) = std::fs::read_to_string(&log) {
        print!("{text}");
        let _ = std::fs::remove_file(&log);
    }
    Ok(code)
}

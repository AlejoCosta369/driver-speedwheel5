//! Helper mínimo sobre la API de registro con control explícito de la vista WOW64.

use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS, WIN32_ERROR};
use windows::Win32::System::Registry::{
    HKEY, KEY_READ, KEY_WOW64_32KEY, KEY_WOW64_64KEY, KEY_WRITE, REG_BINARY, REG_DWORD, REG_OPTION_NON_VOLATILE,
    REG_SAM_FLAGS, REG_SZ, REG_VALUE_TYPE, RegCloseKey, RegCreateKeyExW, RegDeleteTreeW, RegOpenKeyExW,
    RegQueryValueExW, RegSetValueExW,
};
use windows::core::{Error, PCWSTR, Result};

pub use windows::Win32::System::Registry::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};

use crate::wide;

/// Vista del registro. En Windows de 64 bits, `SOFTWARE\Classes\CLSID` existe dos
/// veces: la nativa (DLL de 64 bits) y `WOW6432Node` (DLL de 32 bits).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    Bits64,
    Bits32,
}

impl View {
    fn flag(self) -> REG_SAM_FLAGS {
        match self {
            View::Bits64 => KEY_WOW64_64KEY,
            View::Bits32 => KEY_WOW64_32KEY,
        }
    }
}

/// Lectura + escritura + borrado, sin WRITE_DAC/WRITE_OWNER: `KEY_ALL_ACCESS` falla
/// en claves cuya ACL no da control total (como las OEM que dejó el instalador
/// original en HKCU).
const READ_WRITE: REG_SAM_FLAGS = REG_SAM_FLAGS(KEY_READ.0 | KEY_WRITE.0 | 0x0001_0000 /* DELETE */);

fn check(e: WIN32_ERROR) -> Result<()> {
    if e == ERROR_SUCCESS { Ok(()) } else { Err(Error::from(e.to_hresult())) }
}

pub struct Key(HKEY);

impl Drop for Key {
    fn drop(&mut self) {
        unsafe {
            let _ = RegCloseKey(self.0);
        }
    }
}

impl Key {
    /// Crea (o abre) `root\path` con lectura y escritura.
    pub fn create(root: HKEY, path: &str, view: View) -> Result<Key> {
        let mut h = HKEY::default();
        let p = wide(path);
        check(unsafe {
            RegCreateKeyExW(
                root,
                PCWSTR(p.as_ptr()),
                None,
                PCWSTR::null(),
                REG_OPTION_NON_VOLATILE,
                READ_WRITE | view.flag(),
                None,
                &mut h,
                None,
            )
        })?;
        Ok(Key(h))
    }

    /// Abre `root\path`. `Ok(None)` si no existe.
    pub fn open(root: HKEY, path: &str, view: View, write: bool) -> Result<Option<Key>> {
        let mut h = HKEY::default();
        let p = wide(path);
        let access = if write { READ_WRITE } else { KEY_READ };
        let e = unsafe { RegOpenKeyExW(root, PCWSTR(p.as_ptr()), None, access | view.flag(), &mut h) };
        if e == ERROR_FILE_NOT_FOUND {
            return Ok(None);
        }
        check(e)?;
        Ok(Some(Key(h)))
    }

    pub fn subkey(&self, path: &str) -> Result<Key> {
        let mut h = HKEY::default();
        let p = wide(path);
        check(unsafe {
            RegCreateKeyExW(
                self.0,
                PCWSTR(p.as_ptr()),
                None,
                PCWSTR::null(),
                REG_OPTION_NON_VOLATILE,
                READ_WRITE,
                None,
                &mut h,
                None,
            )
        })?;
        Ok(Key(h))
    }

    fn set_raw(&self, name: &str, ty: REG_VALUE_TYPE, data: &[u8]) -> Result<()> {
        let n = wide(name);
        let name = if name.is_empty() { PCWSTR::null() } else { PCWSTR(n.as_ptr()) };
        check(unsafe { RegSetValueExW(self.0, name, None, ty, Some(data)) })
    }

    /// `name = ""` escribe el valor `(Default)`.
    pub fn set_sz(&self, name: &str, value: &str) -> Result<()> {
        let w = wide(value);
        let bytes: Vec<u8> = w.iter().flat_map(|c| c.to_le_bytes()).collect();
        self.set_raw(name, REG_SZ, &bytes)
    }

    pub fn set_dword(&self, name: &str, value: u32) -> Result<()> {
        self.set_raw(name, REG_DWORD, &value.to_le_bytes())
    }

    pub fn set_binary(&self, name: &str, value: &[u8]) -> Result<()> {
        self.set_raw(name, REG_BINARY, value)
    }

    fn get_raw(&self, name: &str) -> Result<Option<(REG_VALUE_TYPE, Vec<u8>)>> {
        let n = wide(name);
        let name = if name.is_empty() { PCWSTR::null() } else { PCWSTR(n.as_ptr()) };
        let mut ty = REG_VALUE_TYPE::default();
        let mut len = 0u32;
        let e = unsafe { RegQueryValueExW(self.0, name, None, Some(&mut ty), None, Some(&mut len)) };
        if e == ERROR_FILE_NOT_FOUND {
            return Ok(None);
        }
        check(e)?;
        let mut buf = vec![0u8; len as usize];
        check(unsafe { RegQueryValueExW(self.0, name, None, Some(&mut ty), Some(buf.as_mut_ptr()), Some(&mut len)) })?;
        buf.truncate(len as usize);
        Ok(Some((ty, buf)))
    }

    pub fn get_dword(&self, name: &str) -> Result<Option<u32>> {
        Ok(match self.get_raw(name)? {
            Some((ty, b)) if ty == REG_DWORD && b.len() >= 4 => Some(u32::from_le_bytes([b[0], b[1], b[2], b[3]])),
            _ => None,
        })
    }

    pub fn get_sz(&self, name: &str) -> Result<Option<String>> {
        Ok(match self.get_raw(name)? {
            Some((_, b)) => {
                let w: Vec<u16> = b.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
                Some(crate::from_wide(&w))
            }
            None => None,
        })
    }

    /// Borra `self\sub` con todo su contenido. No falla si no existe.
    pub fn delete_tree(&self, sub: &str) -> Result<()> {
        let s = wide(sub);
        let e = unsafe { RegDeleteTreeW(self.0, PCWSTR(s.as_ptr())) };
        if e == ERROR_FILE_NOT_FOUND { Ok(()) } else { check(e) }
    }
}

/// Borra `root\parent\sub` en la vista indicada. No falla si no existe.
pub fn delete_tree(root: HKEY, parent: &str, sub: &str, view: View) -> Result<()> {
    match Key::open(root, parent, view, true)? {
        Some(k) => k.delete_tree(sub),
        None => Ok(()),
    }
}

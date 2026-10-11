//! Acceso HID en modo usuario: enumeración, capacidades y E/S overlapped con timeout.

use std::sync::Mutex;

use windows::Win32::Devices::DeviceAndDriverInstallation::{
    CM_GET_DEVICE_INTERFACE_LIST_PRESENT, CM_Get_Device_Interface_List_SizeW, CM_Get_Device_Interface_ListW,
    CR_BUFFER_SMALL, CR_SUCCESS,
};
use windows::Win32::Devices::HumanInterfaceDevice::{
    HIDD_ATTRIBUTES, HIDP_BUTTON_CAPS, HIDP_CAPS, HIDP_REPORT_TYPE, HIDP_STATUS_SUCCESS, HIDP_VALUE_CAPS,
    HidD_FreePreparsedData, HidD_GetAttributes, HidD_GetHidGuid, HidD_GetPreparsedData, HidD_GetProductString,
    HidP_Feature, HidP_GetButtonCaps, HidP_GetCaps, HidP_GetValueCaps, HidP_Input, HidP_Output, PHIDP_PREPARSED_DATA,
};
use windows::Win32::Foundation::{
    CloseHandle, E_FAIL, ERROR_IO_PENDING, GENERIC_READ, GENERIC_WRITE, HANDLE, WAIT_TIMEOUT,
};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAG_OVERLAPPED, FILE_FLAGS_AND_ATTRIBUTES, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
    ReadFile, WriteFile,
};
use windows::Win32::System::IO::{CancelIoEx, GetOverlappedResultEx, OVERLAPPED};
use windows::Win32::System::Threading::CreateEventW;
use windows::core::{Error, HRESULT, PCWSTR, Result};

use crate::{PIDS, VID, from_wide};

/// Un campo (value o button caps) de un report HID, simplificado.
#[derive(Debug, Clone)]
pub struct FieldCaps {
    pub report_id: u8,
    pub usage_page: u16,
    pub usage_min: u16,
    pub usage_max: u16,
    /// Solo para value caps; 1 para botones.
    pub bit_size: u16,
    pub report_count: u16,
    pub logical_min: i32,
    pub logical_max: i32,
}

#[derive(Debug, Clone, Default)]
pub struct Caps {
    pub usage_page: u16,
    pub usage: u16,
    pub input_len: u16,
    pub output_len: u16,
    pub feature_len: u16,
    pub input_values: Vec<FieldCaps>,
    pub input_buttons: Vec<FieldCaps>,
    pub output_values: Vec<FieldCaps>,
    pub output_buttons: Vec<FieldCaps>,
    pub feature_values: Vec<FieldCaps>,
}

#[derive(Debug, Clone)]
pub struct DeviceInfo {
    /// Ruta de la interfaz (UTF-16 terminada en NUL), apta para `CreateFileW`.
    pub path: Vec<u16>,
    pub vid: u16,
    pub pid: u16,
    pub version: u16,
    pub product: String,
    pub caps: Caps,
}

impl DeviceInfo {
    pub fn path_string(&self) -> String {
        from_wide(&self.path)
    }
}

/// Owned HANDLE que se cierra en `Drop`.
struct OwnedHandle(HANDLE);

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }
}

/// Todas las interfaces HID presentes, como rutas UTF-16 terminadas en NUL.
pub fn hid_interface_paths() -> Result<Vec<Vec<u16>>> {
    let guid = unsafe { HidD_GetHidGuid() };
    loop {
        let mut len = 0u32;
        let cr = unsafe {
            CM_Get_Device_Interface_List_SizeW(&mut len, &guid, PCWSTR::null(), CM_GET_DEVICE_INTERFACE_LIST_PRESENT)
        };
        if cr != CR_SUCCESS {
            return Err(Error::new(E_FAIL, format!("CM_Get_Device_Interface_List_SizeW: {cr:?}")));
        }
        let mut buf = vec![0u16; len as usize];
        let cr = unsafe {
            CM_Get_Device_Interface_ListW(&guid, PCWSTR::null(), &mut buf, CM_GET_DEVICE_INTERFACE_LIST_PRESENT)
        };
        if cr == CR_BUFFER_SMALL {
            continue; // apareció un dispositivo entre las dos llamadas
        }
        if cr != CR_SUCCESS {
            return Err(Error::new(E_FAIL, format!("CM_Get_Device_Interface_ListW: {cr:?}")));
        }
        return Ok(buf
            .split(|&c| c == 0)
            .filter(|s| !s.is_empty())
            .map(|s| s.iter().copied().chain(Some(0)).collect())
            .collect());
    }
}

fn open_handle(path: &[u16], access: u32, flags: FILE_FLAGS_AND_ATTRIBUTES) -> Result<OwnedHandle> {
    let h = unsafe {
        CreateFileW(
            PCWSTR(path.as_ptr()),
            access,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            None,
            OPEN_EXISTING,
            flags,
            None,
        )?
    };
    Ok(OwnedHandle(h))
}

/// Lee atributos, nombre de producto y capacidades de una interfaz HID.
/// Abre el dispositivo sin acceso de lectura/escritura (suficiente para consultas).
pub fn query(path: &[u16]) -> Result<DeviceInfo> {
    let h = open_handle(path, 0, FILE_FLAGS_AND_ATTRIBUTES(0))?;
    let mut attrs = HIDD_ATTRIBUTES { Size: size_of::<HIDD_ATTRIBUTES>() as u32, ..Default::default() };
    if !unsafe { HidD_GetAttributes(h.0, &mut attrs) } {
        return Err(Error::from_thread());
    }
    let mut name = [0u16; 128];
    let product = if unsafe { HidD_GetProductString(h.0, name.as_mut_ptr().cast(), size_of_val(&name) as u32) } {
        from_wide(&name)
    } else {
        String::new()
    };
    let caps = read_caps(h.0)?;
    Ok(DeviceInfo {
        path: path.to_vec(),
        vid: attrs.VendorID,
        pid: attrs.ProductID,
        version: attrs.VersionNumber,
        product,
        caps,
    })
}

fn read_caps(h: HANDLE) -> Result<Caps> {
    let mut ppd = PHIDP_PREPARSED_DATA::default();
    if !unsafe { HidD_GetPreparsedData(h, &mut ppd) } {
        return Err(Error::from_thread());
    }
    struct Ppd(PHIDP_PREPARSED_DATA);
    impl Drop for Ppd {
        fn drop(&mut self) {
            unsafe {
                let _ = HidD_FreePreparsedData(self.0);
            }
        }
    }
    let ppd = Ppd(ppd);

    let mut c = HIDP_CAPS::default();
    if unsafe { HidP_GetCaps(ppd.0, &mut c) } != HIDP_STATUS_SUCCESS {
        return Err(Error::new(E_FAIL, "HidP_GetCaps falló"));
    }
    Ok(Caps {
        usage_page: c.UsagePage,
        usage: c.Usage,
        input_len: c.InputReportByteLength,
        output_len: c.OutputReportByteLength,
        feature_len: c.FeatureReportByteLength,
        input_values: value_caps(HidP_Input, c.NumberInputValueCaps, ppd.0),
        input_buttons: button_caps(HidP_Input, c.NumberInputButtonCaps, ppd.0),
        output_values: value_caps(HidP_Output, c.NumberOutputValueCaps, ppd.0),
        output_buttons: button_caps(HidP_Output, c.NumberOutputButtonCaps, ppd.0),
        feature_values: value_caps(HidP_Feature, c.NumberFeatureValueCaps, ppd.0),
    })
}

fn value_caps(kind: HIDP_REPORT_TYPE, n: u16, ppd: PHIDP_PREPARSED_DATA) -> Vec<FieldCaps> {
    if n == 0 {
        return Vec::new();
    }
    let mut v = vec![HIDP_VALUE_CAPS::default(); n as usize];
    let mut len = n;
    if unsafe { HidP_GetValueCaps(kind, v.as_mut_ptr(), &mut len, ppd) } != HIDP_STATUS_SUCCESS {
        return Vec::new();
    }
    v.truncate(len as usize);
    v.iter()
        .map(|c| {
            let (lo, hi) = unsafe {
                if c.IsRange {
                    (c.Anonymous.Range.UsageMin, c.Anonymous.Range.UsageMax)
                } else {
                    (c.Anonymous.NotRange.Usage, c.Anonymous.NotRange.Usage)
                }
            };
            FieldCaps {
                report_id: c.ReportID,
                usage_page: c.UsagePage,
                usage_min: lo,
                usage_max: hi,
                bit_size: c.BitSize,
                report_count: c.ReportCount,
                logical_min: c.LogicalMin,
                logical_max: c.LogicalMax,
            }
        })
        .collect()
}

fn button_caps(kind: HIDP_REPORT_TYPE, n: u16, ppd: PHIDP_PREPARSED_DATA) -> Vec<FieldCaps> {
    if n == 0 {
        return Vec::new();
    }
    let mut v = vec![HIDP_BUTTON_CAPS::default(); n as usize];
    let mut len = n;
    if unsafe { HidP_GetButtonCaps(kind, v.as_mut_ptr(), &mut len, ppd) } != HIDP_STATUS_SUCCESS {
        return Vec::new();
    }
    v.truncate(len as usize);
    v.iter()
        .map(|c| {
            let (lo, hi) = unsafe {
                if c.IsRange {
                    (c.Anonymous.Range.UsageMin, c.Anonymous.Range.UsageMax)
                } else {
                    (c.Anonymous.NotRange.Usage, c.Anonymous.NotRange.Usage)
                }
            };
            FieldCaps {
                report_id: c.ReportID,
                usage_page: c.UsagePage,
                usage_min: lo,
                usage_max: hi,
                bit_size: 1,
                report_count: hi - lo + 1,
                logical_min: 0,
                logical_max: 1,
            }
        })
        .collect()
}

/// Todos los Speed Wheel 5 Pro conectados.
pub fn find_wheels() -> Result<Vec<DeviceInfo>> {
    Ok(hid_interface_paths()?
        .iter()
        .filter_map(|p| query(p).ok())
        .filter(|d| d.vid == VID && PIDS.contains(&d.pid))
        .collect())
}

/// Dispositivo HID abierto para E/S overlapped. Seguro de compartir entre hilos:
/// cada dirección (lectura / escritura) tiene su propio evento y su propio lock.
pub struct HidDevice {
    handle: OwnedHandle,
    write_event: OwnedHandle,
    read_event: OwnedHandle,
    write_lock: Mutex<()>,
    read_lock: Mutex<()>,
    pub caps: Caps,
}

// SAFETY: los HANDLE de archivo y de evento pueden usarse desde cualquier hilo;
// las operaciones que comparten un OVERLAPPED están serializadas por los Mutex.
unsafe impl Send for HidDevice {}
unsafe impl Sync for HidDevice {}

impl HidDevice {
    pub fn open(path: &[u16]) -> Result<Self> {
        let handle = open_handle(path, GENERIC_READ.0 | GENERIC_WRITE.0, FILE_FLAG_OVERLAPPED)?;
        let caps = read_caps(handle.0)?;
        let write_event = OwnedHandle(unsafe { CreateEventW(None, true, false, PCWSTR::null())? });
        let read_event = OwnedHandle(unsafe { CreateEventW(None, true, false, PCWSTR::null())? });
        Ok(Self { handle, write_event, read_event, write_lock: Mutex::new(()), read_lock: Mutex::new(()), caps })
    }

    /// Escribe un output report (byte 0 = Report ID). Se rellena con ceros hasta
    /// `OutputReportByteLength`. Si no completa en `timeout_ms`, se cancela.
    pub fn write_report(&self, data: &[u8], timeout_ms: u32) -> Result<()> {
        let _g = self.write_lock.lock().unwrap_or_else(|e| e.into_inner());
        let len = (self.caps.output_len as usize).max(data.len());
        let mut buf = vec![0u8; len];
        buf[..data.len()].copy_from_slice(data);
        self.overlapped_io(self.write_event.0, timeout_ms, |ov| unsafe {
            WriteFile(self.handle.0, Some(&buf), None, Some(ov))
        })
        .map(|_| ())
    }

    /// Lee un input report. `Ok(None)` si no llegó nada en `timeout_ms`.
    pub fn read_report(&self, buf: &mut [u8], timeout_ms: u32) -> Result<Option<usize>> {
        let _g = self.read_lock.lock().unwrap_or_else(|e| e.into_inner());
        match self.overlapped_io(self.read_event.0, timeout_ms, |ov| unsafe {
            ReadFile(self.handle.0, Some(buf), None, Some(ov))
        }) {
            Ok(n) => Ok(Some(n)),
            Err(e) if e.code() == HRESULT::from_win32(WAIT_TIMEOUT.0) => Ok(None),
            Err(e) => Err(e),
        }
    }

    fn overlapped_io(
        &self,
        event: HANDLE,
        timeout_ms: u32,
        start: impl FnOnce(*mut OVERLAPPED) -> Result<()>,
    ) -> Result<usize> {
        let mut ov = OVERLAPPED { hEvent: event, ..Default::default() };
        match start(&mut ov) {
            Ok(()) => {}
            Err(e) if e.code() == ERROR_IO_PENDING.to_hresult() => {}
            Err(e) => return Err(e),
        }
        let mut n = 0u32;
        match unsafe { GetOverlappedResultEx(self.handle.0, &ov, &mut n, timeout_ms, false) } {
            Ok(()) => Ok(n as usize),
            Err(e) => {
                // El buffer y el OVERLAPPED viven en este stack frame: hay que esperar
                // a que la cancelación termine antes de salir.
                unsafe {
                    let _ = CancelIoEx(self.handle.0, Some(&ov));
                    let _ = GetOverlappedResultEx(self.handle.0, &ov, &mut n, u32::MAX, false);
                }
                Err(e)
            }
        }
    }
}

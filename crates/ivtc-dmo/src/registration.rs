//! regsvr32 から呼ばれる登録/解除。
//! COMクラス(HKCR\CLSID\{...}\InprocServer32)と、DMOとしての登録(DMORegister)を行う。
//! HKCR(HKLM側)に書くので、regsvr32 は管理者権限で実行すること。

use std::panic::catch_unwind;

use windows::Win32::Foundation::{E_FAIL, GetLastError, HMODULE, S_OK};
use windows::Win32::Media::DxMediaObjects::{
    DMO_PARTIAL_MEDIATYPE, DMOCATEGORY_VIDEO_EFFECT, DMORegister, DMOUnregister,
};
use windows::Win32::System::LibraryLoader::{
    GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS, GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
    GetModuleFileNameW, GetModuleHandleExW,
};
use windows::Win32::System::Registry::{
    HKEY, HKEY_CLASSES_ROOT, KEY_WRITE, REG_OPTION_NON_VOLATILE, REG_SZ, RegCloseKey,
    RegCreateKeyExW, RegDeleteTreeW, RegSetValueExW,
};
use windows::core::{GUID, HRESULT, PCWSTR};

use crate::class_id::{CLSID_PLAYBACK_IVTC_DMO, FRIENDLY_NAME};
use crate::media_type::{MEDIASUBTYPE_NV12, MEDIATYPE_VIDEO};

const NV12_ONLY: [DMO_PARTIAL_MEDIATYPE; 1] = [DMO_PARTIAL_MEDIATYPE {
    r#type: MEDIATYPE_VIDEO,
    subtype: MEDIASUBTYPE_NV12,
}];

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}

/// `{D4B4B989-6B2A-4D0C-8C47-5D3F5CA5A7A1}` 形式(大文字)。
fn guid_braced(g: &GUID) -> String {
    format!(
        "{{{:08X}-{:04X}-{:04X}-{:02X}{:02X}-{:02X}{:02X}{:02X}{:02X}{:02X}{:02X}}}",
        g.data1,
        g.data2,
        g.data3,
        g.data4[0],
        g.data4[1],
        g.data4[2],
        g.data4[3],
        g.data4[4],
        g.data4[5],
        g.data4[6],
        g.data4[7]
    )
}

fn clsid_key() -> String {
    format!(r"CLSID\{}", guid_braced(&CLSID_PLAYBACK_IVTC_DMO))
}

fn write_string(key: HKEY, name: &str, value: &[u16]) -> Result<(), HRESULT> {
    let name = wide(name);
    // REG_SZ のバイト長は終端NULを含める。
    let bytes = unsafe { std::slice::from_raw_parts(value.as_ptr().cast::<u8>(), value.len() * 2) };
    let status = unsafe { RegSetValueExW(key, PCWSTR(name.as_ptr()), None, REG_SZ, Some(bytes)) };
    if status.is_err() {
        Err(status.to_hresult())
    } else {
        Ok(())
    }
}

fn current_dll_path() -> Result<Vec<u16>, HRESULT> {
    let mut module = HMODULE::default();
    let address = register_dmo as *const () as *const u16;
    unsafe {
        GetModuleHandleExW(
            GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
            PCWSTR(address),
            &mut module,
        )
    }
    .map_err(|e| e.code())?;

    let mut buffer = vec![0u16; 32768];
    let length = unsafe { GetModuleFileNameW(Some(module), &mut buffer) } as usize;
    if length == 0 {
        return Err(unsafe { GetLastError() }.to_hresult());
    }
    if length >= buffer.len() {
        return Err(E_FAIL);
    }
    buffer.truncate(length);
    buffer.push(0);
    Ok(buffer)
}

fn register_com_class() -> Result<(), HRESULT> {
    let dll_path = current_dll_path()?;
    let class_key = clsid_key();

    let create = |path: &str| -> Result<HKEY, HRESULT> {
        let subkey = wide(path);
        let mut key = HKEY::default();
        let status = unsafe {
            RegCreateKeyExW(
                HKEY_CLASSES_ROOT,
                PCWSTR(subkey.as_ptr()),
                None,
                PCWSTR::null(),
                REG_OPTION_NON_VOLATILE,
                KEY_WRITE,
                None,
                &mut key,
                None,
            )
        };
        if status.is_err() {
            Err(status.to_hresult())
        } else {
            Ok(key)
        }
    };

    let result = (|| {
        let key = create(&class_key)?;
        let r = write_string(key, "", &wide(FRIENDLY_NAME));
        let _ = unsafe { RegCloseKey(key) };
        r?;

        let key = create(&format!(r"{class_key}\InprocServer32"))?;
        let r = write_string(key, "", &dll_path)
            .and_then(|_| write_string(key, "ThreadingModel", &wide("Both")));
        let _ = unsafe { RegCloseKey(key) };
        r
    })();

    if result.is_err() {
        let _ = remove_com_class();
    }
    result
}

fn remove_com_class() -> Result<(), HRESULT> {
    let subkey = wide(&clsid_key());
    let status = unsafe { RegDeleteTreeW(HKEY_CLASSES_ROOT, PCWSTR(subkey.as_ptr())) };
    if status.is_err() {
        Err(status.to_hresult())
    } else {
        Ok(())
    }
}

fn register_dmo() -> Result<(), HRESULT> {
    register_com_class()?;

    let name = wide(FRIENDLY_NAME);
    // 入出力タイプを登録しておく: グラフ構築側(IFilterMapper2 / MPC-HC)が
    // 「NV12を受けてNV12を出すフィルター」として見つけるために必要。
    let result = unsafe {
        DMORegister(
            PCWSTR(name.as_ptr()),
            &CLSID_PLAYBACK_IVTC_DMO,
            &DMOCATEGORY_VIDEO_EFFECT,
            0,
            NV12_ONLY.len() as u32,
            NV12_ONLY.as_ptr(),
            NV12_ONLY.len() as u32,
            NV12_ONLY.as_ptr(),
        )
    };
    if let Err(error) = result {
        let _ = remove_com_class();
        return Err(error.code());
    }
    Ok(())
}

fn unregister_dmo() -> Result<(), HRESULT> {
    let dmo = unsafe { DMOUnregister(&CLSID_PLAYBACK_IVTC_DMO, &DMOCATEGORY_VIDEO_EFFECT) };
    // DMO登録が既に無くてもCOM側の掃除は試みる。
    let com = remove_com_class();
    dmo.map_err(|e| e.code())?;
    com
}

fn to_hresult(result: std::thread::Result<Result<(), HRESULT>>) -> HRESULT {
    match result {
        Ok(Ok(())) => S_OK,
        Ok(Err(hr)) => hr,
        Err(_) => E_FAIL,
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn DllRegisterServer() -> HRESULT {
    to_hresult(catch_unwind(register_dmo))
}

#[unsafe(no_mangle)]
pub extern "system" fn DllUnregisterServer() -> HRESULT {
    to_hresult(catch_unwind(unregister_dmo))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clsid_registry_path_matches_constant() {
        assert_eq!(clsid_key(), r"CLSID\{D4B4B989-6B2A-4D0C-8C47-5D3F5CA5A7A1}");
    }
}

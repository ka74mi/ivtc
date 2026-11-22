use core::ffi::c_void;

use windows::Win32::Foundation::{CLASS_E_CLASSNOTAVAILABLE, E_POINTER, S_FALSE, S_OK};
use windows::Win32::System::Com::IClassFactory;
use windows::core::{GUID, HRESULT, Interface};

use crate::class_factory::DmoClassFactory;
use crate::class_id::CLSID_PLAYBACK_IVTC_DMO;
use crate::module_state::can_unload;

#[unsafe(no_mangle)]
pub unsafe extern "system" fn DllGetClassObject(
    rclsid: *const GUID,
    riid: *const GUID,
    ppv: *mut *mut c_void,
) -> HRESULT {
    if ppv.is_null() {
        return E_POINTER;
    }
    unsafe { *ppv = core::ptr::null_mut() };
    if rclsid.is_null() || riid.is_null() {
        return E_POINTER;
    }
    if unsafe { *rclsid } != CLSID_PLAYBACK_IVTC_DMO {
        trace!("DllGetClassObject: unknown CLSID {:?}", unsafe { *rclsid });
        return CLASS_E_CLASSNOTAVAILABLE;
    }

    trace!("DllGetClassObject: ok");
    let factory: IClassFactory = DmoClassFactory::new().into();
    unsafe { factory.query(riid, ppv) }
}

#[unsafe(no_mangle)]
pub extern "system" fn DllCanUnloadNow() -> HRESULT {
    if can_unload() { S_OK } else { S_FALSE }
}

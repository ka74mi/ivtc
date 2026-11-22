use core::ffi::c_void;

use windows::Win32::Foundation::E_POINTER;
use windows::Win32::System::Com::{IClassFactory, IClassFactory_Impl};
use windows::core::{BOOL, Error, GUID, IUnknown, Interface, Ref, Result, implement};

use crate::com_object;
use crate::module_state::{ModuleRef, lock_server};

#[implement(IClassFactory)]
pub(crate) struct DmoClassFactory {
    _module_ref: ModuleRef,
}

impl DmoClassFactory {
    pub(crate) fn new() -> Self {
        Self {
            _module_ref: ModuleRef::new(),
        }
    }
}

impl IClassFactory_Impl for DmoClassFactory_Impl {
    fn CreateInstance(
        &self,
        punkouter: Ref<'_, IUnknown>,
        riid: *const GUID,
        ppvobject: *mut *mut c_void,
    ) -> Result<()> {
        if ppvobject.is_null() || riid.is_null() {
            return Err(Error::from_hresult(E_POINTER));
        }
        let outer = punkouter
            .as_ref()
            .map_or(core::ptr::null_mut(), |u| u.as_raw());
        unsafe { com_object::create_instance(outer, riid, ppvobject) }.ok()
    }

    fn LockServer(&self, flock: BOOL) -> Result<()> {
        lock_server(flock.as_bool());
        Ok(())
    }
}

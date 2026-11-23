use core::ffi::c_void;
use core::mem::offset_of;
use core::panic::AssertUnwindSafe;
use core::sync::atomic::{AtomicU32, Ordering, fence};
use std::panic::catch_unwind;
use std::sync::{Mutex, MutexGuard};

use windows::Win32::Foundation::{
    CLASS_E_NOAGGREGATION, E_FAIL, E_INVALIDARG, E_NOINTERFACE, E_NOTIMPL, E_POINTER, S_FALSE, S_OK,
};
use windows::Win32::Media::DxMediaObjects::{
    DMO_E_INVALIDSTREAMINDEX, DMO_E_NO_MORE_ITEMS, DMO_INPUT_STATUSF_ACCEPT_DATA,
    DMO_INPUT_STREAMF_FIXED_SAMPLE_SIZE, DMO_INPUT_STREAMF_SINGLE_SAMPLE_PER_BUFFER,
    DMO_INPUT_STREAMF_WHOLE_SAMPLES, DMO_MEDIA_TYPE, DMO_OUTPUT_DATA_BUFFER,
    DMO_OUTPUT_DATA_BUFFERF_DISCONTINUITY, DMO_OUTPUT_DATA_BUFFERF_INCOMPLETE,
    DMO_OUTPUT_DATA_BUFFERF_SYNCPOINT, DMO_OUTPUT_DATA_BUFFERF_TIME,
    DMO_OUTPUT_DATA_BUFFERF_TIMELENGTH, DMO_OUTPUT_STREAMF_FIXED_SAMPLE_SIZE,
    DMO_OUTPUT_STREAMF_SINGLE_SAMPLE_PER_BUFFER, DMO_OUTPUT_STREAMF_WHOLE_SAMPLES,
    DMO_PROCESS_OUTPUT_DISCARD_WHEN_NO_BUFFER, DMO_SET_TYPEF_CLEAR, DMO_SET_TYPEF_TEST_ONLY,
    IMediaBuffer, IMediaObject, IMediaObject_Vtbl,
};
use windows::core::{GUID, HRESULT, IUnknown, IUnknown_Vtbl, Interface};

use crate::media_type::{NegotiatedType, write_partial_nv12};
use crate::module_state::ModuleRef;
use crate::processor::{DmoResult, Processor, Status};

#[repr(C)]
struct MediaObjectSlot {
    vtbl: *const IMediaObject_Vtbl,
}

#[repr(C)]
struct InnerUnknownSlot {
    vtbl: *const IUnknown_Vtbl,
}

#[repr(C)]
struct DmoObject {
    media_object: MediaObjectSlot,

    inner_unknown: InnerUnknownSlot,

    refs: AtomicU32,

    outer: *mut c_void,
    processor: Mutex<Processor>,
    _module: ModuleRef,
}

fn media_object_ptr(obj: *mut DmoObject) -> *mut c_void {
    obj.cast()
}

fn inner_unknown_ptr(obj: *mut DmoObject) -> *mut c_void {
    unsafe {
        obj.cast::<u8>()
            .add(offset_of!(DmoObject, inner_unknown))
            .cast()
    }
}

fn obj_from_media_object(this: *mut c_void) -> *mut DmoObject {
    this.cast()
}

fn obj_from_inner_unknown(this: *mut c_void) -> *mut DmoObject {
    unsafe {
        this.cast::<u8>()
            .sub(offset_of!(DmoObject, inner_unknown))
            .cast()
    }
}

unsafe fn outer_vtbl<'a>(outer: *mut c_void) -> &'a IUnknown_Vtbl {
    unsafe { &**outer.cast::<*const IUnknown_Vtbl>() }
}

unsafe fn release_own(obj: *mut DmoObject) -> u32 {
    let remaining = unsafe { (*obj).refs.fetch_sub(1, Ordering::Release) } - 1;
    if remaining == 0 {
        fence(Ordering::Acquire);
        trace!("DmoObject destroyed");
        drop(unsafe { Box::from_raw(obj) });
    }
    remaining
}

unsafe fn controlling_add_ref(obj: *mut DmoObject) -> u32 {
    let outer = unsafe { (*obj).outer };
    if outer.is_null() {
        unsafe { (*obj).refs.fetch_add(1, Ordering::Relaxed) + 1 }
    } else {
        unsafe { (outer_vtbl(outer).AddRef)(outer) }
    }
}

unsafe fn controlling_release(obj: *mut DmoObject) -> u32 {
    let outer = unsafe { (*obj).outer };
    if outer.is_null() {
        unsafe { release_own(obj) }
    } else {
        unsafe { (outer_vtbl(outer).Release)(outer) }
    }
}

unsafe fn inner_query(obj: *mut DmoObject, iid: *const GUID, out: *mut *mut c_void) -> HRESULT {
    if iid.is_null() || out.is_null() {
        return E_POINTER;
    }
    unsafe { *out = core::ptr::null_mut() };
    let iid = unsafe { *iid };

    if iid == IUnknown::IID {
        unsafe {
            *out = inner_unknown_ptr(obj);
            (*obj).refs.fetch_add(1, Ordering::Relaxed);
        }
        S_OK
    } else if iid == IMediaObject::IID {
        unsafe {
            *out = media_object_ptr(obj);
            controlling_add_ref(obj);
        }
        S_OK
    } else {
        trace!("QueryInterface: unsupported IID {iid:?}");
        E_NOINTERFACE
    }
}

unsafe extern "system" fn inner_query_interface(
    this: *mut c_void,
    iid: *const GUID,
    out: *mut *mut c_void,
) -> HRESULT {
    unsafe { inner_query(obj_from_inner_unknown(this), iid, out) }
}
unsafe extern "system" fn inner_add_ref(this: *mut c_void) -> u32 {
    unsafe {
        (*obj_from_inner_unknown(this))
            .refs
            .fetch_add(1, Ordering::Relaxed)
            + 1
    }
}
unsafe extern "system" fn inner_release(this: *mut c_void) -> u32 {
    unsafe { release_own(obj_from_inner_unknown(this)) }
}

unsafe extern "system" fn mo_query_interface(
    this: *mut c_void,
    iid: *const GUID,
    out: *mut *mut c_void,
) -> HRESULT {
    let obj = obj_from_media_object(this);
    let outer = unsafe { (*obj).outer };
    if outer.is_null() {
        unsafe { inner_query(obj, iid, out) }
    } else {
        unsafe { (outer_vtbl(outer).QueryInterface)(outer, iid, out) }
    }
}
unsafe extern "system" fn mo_add_ref(this: *mut c_void) -> u32 {
    unsafe { controlling_add_ref(obj_from_media_object(this)) }
}
unsafe extern "system" fn mo_release(this: *mut c_void) -> u32 {
    unsafe { controlling_release(obj_from_media_object(this)) }
}

static INNER_UNKNOWN_VTBL: IUnknown_Vtbl = IUnknown_Vtbl {
    QueryInterface: inner_query_interface,
    AddRef: inner_add_ref,
    Release: inner_release,
};

/// # Safety
/// `riid` と `ppv` は有効なポインタ、`outer` は null か有効な `IUnknown` を指すこと。
pub(crate) unsafe fn create_instance(
    outer: *mut c_void,
    riid: *const GUID,
    ppv: *mut *mut c_void,
) -> HRESULT {
    unsafe { *ppv = core::ptr::null_mut() };
    if !outer.is_null() && unsafe { *riid } != IUnknown::IID {
        return CLASS_E_NOAGGREGATION;
    }

    trace!("CreateInstance aggregated={}", !outer.is_null());
    let obj = Box::into_raw(Box::new(DmoObject {
        media_object: MediaObjectSlot {
            vtbl: &MEDIA_OBJECT_VTBL,
        },
        inner_unknown: InnerUnknownSlot {
            vtbl: &INNER_UNKNOWN_VTBL,
        },
        refs: AtomicU32::new(1),
        outer,
        processor: Mutex::new(Processor::new()),
        _module: ModuleRef::new(),
    }));

    if !outer.is_null() {
        unsafe { *ppv = inner_unknown_ptr(obj) };
        return S_OK;
    }
    let hr = unsafe { inner_query(obj, riid, ppv) };
    unsafe { release_own(obj) };
    hr
}

fn run(
    this: *mut c_void,
    name: &'static str,
    f: impl FnOnce(&mut Processor) -> DmoResult<Status>,
) -> HRESULT {
    trace!("> {name}");
    let obj = obj_from_media_object(this);
    let result = catch_unwind(AssertUnwindSafe(|| {
        let mutex = unsafe { &(*obj).processor };
        let mut guard: MutexGuard<'_, Processor> = mutex.lock().unwrap_or_else(|e| e.into_inner());
        f(&mut guard)
    }));
    let hr = match result {
        Ok(Ok(Status::Done)) => S_OK,
        Ok(Ok(Status::NoOutput)) => S_FALSE,
        Ok(Err(hr)) => hr,
        Err(_) => {
            trace!("! {name} panicked");
            E_FAIL
        }
    };
    trace!("< {name} -> {:#010x}", hr.0 as u32);
    hr
}

fn check_stream(index: u32) -> DmoResult<()> {
    if index == 0 {
        Ok(())
    } else {
        Err(DMO_E_INVALIDSTREAMINDEX)
    }
}

fn write_u32(p: *mut u32, v: u32) -> DmoResult<()> {
    if p.is_null() {
        return Err(E_POINTER);
    }
    unsafe { *p = v };
    Ok(())
}

unsafe extern "system" fn get_stream_count(
    this: *mut c_void,
    pin: *mut u32,
    pout: *mut u32,
) -> HRESULT {
    run(this, "GetStreamCount", |_| {
        write_u32(pin, 1)?;
        write_u32(pout, 1)?;
        Ok(Status::Done)
    })
}

unsafe extern "system" fn get_input_stream_info(
    this: *mut c_void,
    index: u32,
    flags: *mut u32,
) -> HRESULT {
    run(this, "GetInputStreamInfo", |_| {
        check_stream(index)?;
        write_u32(
            flags,
            (DMO_INPUT_STREAMF_WHOLE_SAMPLES.0
                | DMO_INPUT_STREAMF_SINGLE_SAMPLE_PER_BUFFER.0
                | DMO_INPUT_STREAMF_FIXED_SAMPLE_SIZE.0) as u32,
        )?;
        Ok(Status::Done)
    })
}

unsafe extern "system" fn get_output_stream_info(
    this: *mut c_void,
    index: u32,
    flags: *mut u32,
) -> HRESULT {
    run(this, "GetOutputStreamInfo", |_| {
        check_stream(index)?;
        write_u32(
            flags,
            (DMO_OUTPUT_STREAMF_WHOLE_SAMPLES.0
                | DMO_OUTPUT_STREAMF_SINGLE_SAMPLE_PER_BUFFER.0
                | DMO_OUTPUT_STREAMF_FIXED_SAMPLE_SIZE.0) as u32,
        )?;
        Ok(Status::Done)
    })
}

unsafe extern "system" fn get_input_type(
    this: *mut c_void,
    index: u32,
    type_index: u32,
    pmt: *mut DMO_MEDIA_TYPE,
) -> HRESULT {
    run(this, "GetInputType", |_| {
        check_stream(index)?;
        trace!("  type_index={type_index} pmt_null={}", pmt.is_null());
        if type_index != 0 {
            return Err(DMO_E_NO_MORE_ITEMS);
        }
        if pmt.is_null() {
            return Ok(Status::Done);
        }
        unsafe { write_partial_nv12(pmt) }?;
        Ok(Status::Done)
    })
}

unsafe extern "system" fn get_output_type(
    this: *mut c_void,
    index: u32,
    type_index: u32,
    pmt: *mut DMO_MEDIA_TYPE,
) -> HRESULT {
    run(this, "GetOutputType", |p| {
        check_stream(index)?;
        trace!("  type_index={type_index} pmt_null={}", pmt.is_null());
        let preferred = p.preferred_output_type()?;
        if type_index != 0 {
            return Err(DMO_E_NO_MORE_ITEMS);
        }
        if pmt.is_null() {
            return Ok(Status::Done);
        }
        unsafe { preferred.write_to(pmt) }?;
        Ok(Status::Done)
    })
}

fn set_type_common(
    pmt: *const DMO_MEDIA_TYPE,
    flags: u32,
    clear: impl FnOnce(),
    set: impl FnOnce(NegotiatedType, bool) -> DmoResult<()>,
) -> DmoResult<Status> {
    let known = (DMO_SET_TYPEF_TEST_ONLY.0 | DMO_SET_TYPEF_CLEAR.0) as u32;
    if flags & !known != 0 {
        return Err(E_INVALIDARG);
    }
    if flags & DMO_SET_TYPEF_CLEAR.0 as u32 != 0 {
        if flags & DMO_SET_TYPEF_TEST_ONLY.0 as u32 == 0 {
            clear();
        }
        return Ok(Status::Done);
    }
    let nt = unsafe { NegotiatedType::from_raw(pmt) }?;
    set(nt, flags & DMO_SET_TYPEF_TEST_ONLY.0 as u32 != 0)?;
    Ok(Status::Done)
}

unsafe extern "system" fn set_input_type(
    this: *mut c_void,
    index: u32,
    pmt: *const DMO_MEDIA_TYPE,
    flags: u32,
) -> HRESULT {
    run(this, "SetInputType", |p| {
        check_stream(index)?;
        trace!("  flags={flags:#x}");
        let p = core::cell::RefCell::new(p);
        set_type_common(
            pmt,
            flags,
            || p.borrow_mut().clear_input(),
            |nt, test| p.borrow_mut().set_input_type(nt, test),
        )
    })
}

unsafe extern "system" fn set_output_type(
    this: *mut c_void,
    index: u32,
    pmt: *const DMO_MEDIA_TYPE,
    flags: u32,
) -> HRESULT {
    run(this, "SetOutputType", |p| {
        check_stream(index)?;
        trace!("  flags={flags:#x}");
        let p = core::cell::RefCell::new(p);
        set_type_common(
            pmt,
            flags,
            || p.borrow_mut().clear_output(),
            |nt, test| p.borrow_mut().set_output_type(nt, test),
        )
    })
}

unsafe extern "system" fn get_input_current_type(
    this: *mut c_void,
    index: u32,
    pmt: *mut DMO_MEDIA_TYPE,
) -> HRESULT {
    run(this, "GetInputCurrentType", |p| {
        check_stream(index)?;
        unsafe { p.input_type()?.write_to(pmt) }?;
        Ok(Status::Done)
    })
}

unsafe extern "system" fn get_output_current_type(
    this: *mut c_void,
    index: u32,
    pmt: *mut DMO_MEDIA_TYPE,
) -> HRESULT {
    run(this, "GetOutputCurrentType", |p| {
        check_stream(index)?;
        unsafe { p.output_type()?.write_to(pmt) }?;
        Ok(Status::Done)
    })
}

unsafe extern "system" fn get_input_size_info(
    this: *mut c_void,
    index: u32,
    size: *mut u32,
    lookahead: *mut u32,
    alignment: *mut u32,
) -> HRESULT {
    run(this, "GetInputSizeInfo", |p| {
        check_stream(index)?;
        let n = p.input_type()?.frame_size() as u32;
        write_u32(size, n)?;
        write_u32(lookahead, 0)?;
        write_u32(alignment, 1)?;
        trace!("  size={n}");
        Ok(Status::Done)
    })
}

unsafe extern "system" fn get_output_size_info(
    this: *mut c_void,
    index: u32,
    size: *mut u32,
    alignment: *mut u32,
) -> HRESULT {
    run(this, "GetOutputSizeInfo", |p| {
        check_stream(index)?;
        let n = p.output_type()?.frame_size() as u32;
        write_u32(size, n)?;
        write_u32(alignment, 1)?;
        trace!("  size={n}");
        Ok(Status::Done)
    })
}

unsafe extern "system" fn get_input_max_latency(
    this: *mut c_void,
    _index: u32,
    _latency: *mut i64,
) -> HRESULT {
    run(this, "GetInputMaxLatency", |_| Err(E_NOTIMPL))
}

unsafe extern "system" fn set_input_max_latency(
    this: *mut c_void,
    _index: u32,
    _latency: i64,
) -> HRESULT {
    run(this, "SetInputMaxLatency", |_| Err(E_NOTIMPL))
}

unsafe extern "system" fn flush(this: *mut c_void) -> HRESULT {
    run(this, "Flush", |p| {
        p.flush();
        Ok(Status::Done)
    })
}

unsafe extern "system" fn discontinuity(this: *mut c_void, index: u32) -> HRESULT {
    run(this, "Discontinuity", |p| {
        check_stream(index)?;
        p.discontinuity()?;
        Ok(Status::Done)
    })
}

unsafe extern "system" fn allocate_streaming_resources(this: *mut c_void) -> HRESULT {
    run(this, "AllocateStreamingResources", |_| Ok(Status::Done))
}

unsafe extern "system" fn free_streaming_resources(this: *mut c_void) -> HRESULT {
    run(this, "FreeStreamingResources", |p| {
        p.free_resources();
        Ok(Status::Done)
    })
}

unsafe extern "system" fn get_input_status(
    this: *mut c_void,
    index: u32,
    flags: *mut u32,
) -> HRESULT {
    run(this, "GetInputStatus", |p| {
        check_stream(index)?;
        let accept = p.input_status_accepts_data()?;
        write_u32(
            flags,
            if accept {
                DMO_INPUT_STATUSF_ACCEPT_DATA.0 as u32
            } else {
                0
            },
        )?;
        Ok(Status::Done)
    })
}

unsafe extern "system" fn process_input(
    this: *mut c_void,
    index: u32,
    buffer: *mut c_void,
    flags: u32,
    time: i64,
    length: i64,
) -> HRESULT {
    run(this, "ProcessInput", |p| {
        check_stream(index)?;
        let Some(buffer) = (unsafe { IMediaBuffer::from_raw_borrowed(&buffer) }) else {
            return Err(E_POINTER);
        };
        let (mut ptr, mut len) = (core::ptr::null_mut::<u8>(), 0u32);
        unsafe { buffer.GetBufferAndLength(Some(&mut ptr), Some(&mut len)) }
            .map_err(|e| e.code())?;
        trace!("  len={len} flags={flags:#x} time={time} dur={length}");
        let data: &[u8] = if ptr.is_null() || len == 0 {
            &[]
        } else {
            unsafe { core::slice::from_raw_parts(ptr, len as usize) }
        };
        p.process_input(data, flags, time, length)
    })
}

unsafe extern "system" fn process_output(
    this: *mut c_void,
    flags: u32,
    count: u32,
    buffers: *mut DMO_OUTPUT_DATA_BUFFER,
    status: *mut u32,
) -> HRESULT {
    run(this, "ProcessOutput", |p| {
        if buffers.is_null() {
            return Err(E_POINTER);
        }
        if count != 1 {
            return Err(E_INVALIDARG);
        }
        if !status.is_null() {
            unsafe { *status = 0 };
        }
        let slot = unsafe { &mut *buffers };
        slot.dwStatus = 0;

        if !p.has_ready_output() {
            return Ok(Status::NoOutput);
        }

        let Some(buffer) = slot.pBuffer.as_ref() else {
            if flags & DMO_PROCESS_OUTPUT_DISCARD_WHEN_NO_BUFFER.0 as u32 != 0 {
                p.discard_ready();
                return Ok(Status::NoOutput);
            }
            return Err(E_INVALIDARG);
        };

        let need = p.output_frame_size()?;
        let (mut ptr, mut len) = (core::ptr::null_mut::<u8>(), 0u32);
        unsafe { buffer.GetBufferAndLength(Some(&mut ptr), Some(&mut len)) }
            .map_err(|e| e.code())?;
        let max = unsafe { buffer.GetMaxLength() }.map_err(|e| e.code())?;
        trace!("  out buffer len={len} max={max} need={need}");
        if ptr.is_null() || (max as usize) < len as usize + need {
            trace!("  output buffer too small");
            return Err(E_INVALIDARG);
        }

        let dst = unsafe { core::slice::from_raw_parts_mut(ptr.add(len as usize), need) };
        let Some(meta) = p.process_output(dst)? else {
            return Ok(Status::NoOutput);
        };
        unsafe { buffer.SetLength(len + need as u32) }.map_err(|e| e.code())?;

        let mut st = DMO_OUTPUT_DATA_BUFFERF_SYNCPOINT.0 as u32;
        if let Some(t) = meta.time {
            st |= DMO_OUTPUT_DATA_BUFFERF_TIME.0 as u32;
            slot.rtTimestamp = t;
        }
        if let Some(l) = meta.length {
            st |= DMO_OUTPUT_DATA_BUFFERF_TIMELENGTH.0 as u32;
            slot.rtTimelength = l;
        }
        if meta.discontinuity {
            st |= DMO_OUTPUT_DATA_BUFFERF_DISCONTINUITY.0 as u32;
        }
        if meta.more {
            st |= DMO_OUTPUT_DATA_BUFFERF_INCOMPLETE.0 as u32;
        }
        slot.dwStatus = st;
        trace!("  out status={st:#x} time={:?}", meta.time);
        Ok(Status::Done)
    })
}

unsafe extern "system" fn lock(this: *mut c_void, _lock: i32) -> HRESULT {
    run(this, "Lock", |_| Ok(Status::Done))
}

static MEDIA_OBJECT_VTBL: IMediaObject_Vtbl = IMediaObject_Vtbl {
    base__: IUnknown_Vtbl {
        QueryInterface: mo_query_interface,
        AddRef: mo_add_ref,
        Release: mo_release,
    },
    GetStreamCount: get_stream_count,
    GetInputStreamInfo: get_input_stream_info,
    GetOutputStreamInfo: get_output_stream_info,
    GetInputType: get_input_type,
    GetOutputType: get_output_type,
    SetInputType: set_input_type,
    SetOutputType: set_output_type,
    GetInputCurrentType: get_input_current_type,
    GetOutputCurrentType: get_output_current_type,
    GetInputSizeInfo: get_input_size_info,
    GetOutputSizeInfo: get_output_size_info,
    GetInputMaxLatency: get_input_max_latency,
    SetInputMaxLatency: set_input_max_latency,
    Flush: flush,
    Discontinuity: discontinuity,
    AllocateStreamingResources: allocate_streaming_resources,
    FreeStreamingResources: free_streaming_resources,
    GetInputStatus: get_input_status,
    ProcessInput: process_input,
    ProcessOutput: process_output,
    Lock: lock,
};

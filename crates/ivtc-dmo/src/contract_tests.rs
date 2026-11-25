use core::ffi::c_void;
use core::mem::ManuallyDrop;
use std::cell::{Cell, RefCell};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, Ordering};

use ivtc_core::nv12::Nv12Layout;
use ivtc_core::video_info::{BI_NV12, VIDEOINFOHEADER2_SIZE};
use windows::Win32::Foundation::{CLASS_E_NOAGGREGATION, E_NOINTERFACE, S_FALSE, S_OK};
use windows::Win32::Media::DxMediaObjects::{
    DMO_INPUT_DATA_BUFFERF_TIME, DMO_INPUT_STATUSF_ACCEPT_DATA, DMO_MEDIA_TYPE,
    DMO_OUTPUT_DATA_BUFFER, DMO_OUTPUT_DATA_BUFFERF_INCOMPLETE, DMO_OUTPUT_DATA_BUFFERF_TIME,
    IMediaBuffer, IMediaBuffer_Impl, IMediaObject,
};
use windows::core::{BOOL, GUID, HRESULT, IUnknown, IUnknown_Vtbl, Interface, implement};

use crate::com_object::create_instance;
use crate::media_type::{MEDIASUBTYPE_NV12, MEDIATYPE_VIDEO};

const FORMAT_VIDEOINFO2: GUID = GUID::from_u128(0xf72a76a0_eb0a_11d0_ace4_0000c0cc16ba);
const WIDTH: u32 = 64;
const HEIGHT: u32 = 32;

#[implement(IMediaBuffer)]
struct TestBuffer {
    data: RefCell<Vec<u8>>,
    len: Cell<u32>,
}

impl IMediaBuffer_Impl for TestBuffer_Impl {
    fn SetLength(&self, cblength: u32) -> windows::core::Result<()> {
        self.len.set(cblength);
        Ok(())
    }

    fn GetMaxLength(&self) -> windows::core::Result<u32> {
        Ok(self.data.borrow().len() as u32)
    }

    fn GetBufferAndLength(
        &self,
        ppbuffer: *mut *mut u8,
        pcblength: *mut u32,
    ) -> windows::core::Result<()> {
        if !ppbuffer.is_null() {
            unsafe { *ppbuffer = self.data.borrow_mut().as_mut_ptr() };
        }
        if !pcblength.is_null() {
            unsafe { *pcblength = self.len.get() };
        }
        Ok(())
    }
}

fn frame_size() -> usize {
    Nv12Layout::packed(WIDTH, HEIGHT).unwrap().frame_size()
}

fn frame(seed: u8) -> Vec<u8> {
    (0..frame_size())
        .map(|i| (i as u8).wrapping_add(seed))
        .collect()
}

fn make_buffer(capacity: usize, initial: &[u8]) -> IMediaBuffer {
    let mut data = vec![0u8; capacity];
    data[..initial.len()].copy_from_slice(initial);
    TestBuffer {
        data: RefCell::new(data),
        len: Cell::new(initial.len() as u32),
    }
    .into()
}

fn buffer_bytes(buffer: &IMediaBuffer) -> Vec<u8> {
    let (mut ptr, mut len) = (core::ptr::null_mut::<u8>(), 0u32);
    unsafe { buffer.GetBufferAndLength(Some(&mut ptr), Some(&mut len)) }.unwrap();
    unsafe { core::slice::from_raw_parts(ptr, len as usize) }.to_vec()
}

fn format_block() -> Vec<u8> {
    let mut b = vec![0u8; VIDEOINFOHEADER2_SIZE];
    b[40..48].copy_from_slice(&333_667i64.to_le_bytes());
    b[72..76].copy_from_slice(&40u32.to_le_bytes());
    b[76..80].copy_from_slice(&(WIDTH as i32).to_le_bytes());
    b[80..84].copy_from_slice(&(HEIGHT as i32).to_le_bytes());
    b[84..86].copy_from_slice(&1u16.to_le_bytes());
    b[86..88].copy_from_slice(&12u16.to_le_bytes());
    b[88..92].copy_from_slice(&BI_NV12.to_le_bytes());
    b
}

fn media_type(format: &mut [u8]) -> DMO_MEDIA_TYPE {
    DMO_MEDIA_TYPE {
        majortype: MEDIATYPE_VIDEO,
        subtype: MEDIASUBTYPE_NV12,
        bFixedSizeSamples: BOOL(1),
        bTemporalCompression: BOOL(0),
        lSampleSize: frame_size() as u32,
        formattype: FORMAT_VIDEOINFO2,
        pUnk: ManuallyDrop::new(None),
        cbFormat: format.len() as u32,
        pbFormat: format.as_mut_ptr(),
    }
}

static CREATE_LOCK: Mutex<()> = Mutex::new(());

fn create_dmo(delay: usize) -> IMediaObject {
    let _guard = CREATE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    unsafe { std::env::set_var("PLAYBACK_IVTC_DELAY", delay.to_string()) };
    let mut raw = core::ptr::null_mut();
    let hr = unsafe { create_instance(core::ptr::null_mut(), &IMediaObject::IID, &mut raw) };
    unsafe { std::env::remove_var("PLAYBACK_IVTC_DELAY") };
    assert_eq!(hr, S_OK);
    unsafe { IMediaObject::from_raw(raw) }
}

fn negotiated_dmo(delay: usize) -> IMediaObject {
    let mo = create_dmo(delay);
    let mut format = format_block();
    let mt = media_type(&mut format);
    unsafe {
        assert_eq!((mo.vtable().SetInputType)(mo.as_raw(), 0, &mt, 0), S_OK);
        assert_eq!((mo.vtable().SetOutputType)(mo.as_raw(), 0, &mt, 0), S_OK);
    }
    mo
}

fn push(mo: &IMediaObject, seed: u8, time: i64) -> HRESULT {
    let buffer = make_buffer(frame_size(), &frame(seed));
    unsafe {
        (mo.vtable().ProcessInput)(
            mo.as_raw(),
            0,
            buffer.as_raw(),
            DMO_INPUT_DATA_BUFFERF_TIME.0 as u32,
            time,
            0,
        )
    }
}

struct Pulled {
    hr: HRESULT,
    status: u32,
    time: i64,
    bytes: Vec<u8>,
}

fn pull(mo: &IMediaObject, prefix: &[u8]) -> Pulled {
    let buffer = make_buffer(prefix.len() + frame_size(), prefix);
    let mut slot = DMO_OUTPUT_DATA_BUFFER {
        pBuffer: ManuallyDrop::new(Some(buffer.clone())),
        dwStatus: 0,
        rtTimestamp: 0,
        rtTimelength: 0,
    };
    let mut status = 0u32;
    let hr = unsafe { (mo.vtable().ProcessOutput)(mo.as_raw(), 0, 1, &mut slot, &mut status) };
    unsafe { ManuallyDrop::drop(&mut slot.pBuffer) };
    Pulled {
        hr,
        status: slot.dwStatus,
        time: slot.rtTimestamp,
        bytes: buffer_bytes(&buffer),
    }
}

fn discontinuity(mo: &IMediaObject) -> HRESULT {
    unsafe { (mo.vtable().Discontinuity)(mo.as_raw(), 0) }
}

#[repr(C)]
struct Outer {
    vtbl: *const IUnknown_Vtbl,
    adds: AtomicU32,
    releases: AtomicU32,
    queries: AtomicU32,
}

unsafe extern "system" fn outer_query(
    this: *mut c_void,
    _iid: *const GUID,
    out: *mut *mut c_void,
) -> HRESULT {
    unsafe {
        (*this.cast::<Outer>())
            .queries
            .fetch_add(1, Ordering::SeqCst);
        *out = core::ptr::null_mut();
    }
    E_NOINTERFACE
}

unsafe extern "system" fn outer_add_ref(this: *mut c_void) -> u32 {
    unsafe { (*this.cast::<Outer>()).adds.fetch_add(1, Ordering::SeqCst) + 1 }
}

unsafe extern "system" fn outer_release(this: *mut c_void) -> u32 {
    unsafe {
        (*this.cast::<Outer>())
            .releases
            .fetch_add(1, Ordering::SeqCst)
            + 1
    }
}

static OUTER_VTBL: IUnknown_Vtbl = IUnknown_Vtbl {
    QueryInterface: outer_query,
    AddRef: outer_add_ref,
    Release: outer_release,
};

impl Outer {
    fn new() -> Self {
        Self {
            vtbl: &OUTER_VTBL,
            adds: AtomicU32::new(0),
            releases: AtomicU32::new(0),
            queries: AtomicU32::new(0),
        }
    }

    fn as_raw(&self) -> *mut c_void {
        core::ptr::from_ref(self).cast_mut().cast()
    }
}

#[test]
fn aggregation_delegates_controlling_calls_to_outer() {
    let outer = Outer::new();
    let mut raw = core::ptr::null_mut();
    let hr = unsafe { create_instance(outer.as_raw(), &IUnknown::IID, &mut raw) };
    assert_eq!(hr, S_OK);
    let inner = unsafe { IUnknown::from_raw(raw) };

    let mo: IMediaObject = inner.cast().unwrap();
    assert_eq!(outer.adds.load(Ordering::SeqCst), 1);

    assert!(mo.cast::<IMediaBuffer>().is_err());
    assert_eq!(outer.queries.load(Ordering::SeqCst), 1);

    drop(mo);
    assert_eq!(outer.releases.load(Ordering::SeqCst), 1);
}

#[test]
fn aggregation_accepts_only_iunknown() {
    let outer = Outer::new();
    let mut raw = core::ptr::null_mut();
    let hr = unsafe { create_instance(outer.as_raw(), &IMediaObject::IID, &mut raw) };
    assert_eq!(hr, CLASS_E_NOAGGREGATION);
    assert!(raw.is_null());
}

#[test]
fn process_output_without_input_is_s_false() {
    let mo = negotiated_dmo(0);
    let out = pull(&mo, &[]);
    assert_eq!(out.hr, S_FALSE);
    assert_eq!(out.status, 0);
    assert!(out.bytes.is_empty());
}

#[test]
fn process_output_appends_after_existing_buffer_contents() {
    let mo = negotiated_dmo(0);
    assert_eq!(push(&mo, 7, 0), S_OK);
    let prefix = [0xEEu8; 16];
    let out = pull(&mo, &prefix);
    assert_eq!(out.hr, S_OK);
    assert_eq!(&out.bytes[..16], &prefix);
    assert_eq!(&out.bytes[16..], frame(7).as_slice());
}

#[test]
fn end_of_stream_drains_queued_frames_in_order() {
    let incomplete = DMO_OUTPUT_DATA_BUFFERF_INCOMPLETE.0 as u32;
    let has_time = DMO_OUTPUT_DATA_BUFFERF_TIME.0 as u32;
    let mo = negotiated_dmo(2);
    assert_eq!(push(&mo, 1, 100), S_FALSE);
    assert_eq!(push(&mo, 2, 200), S_FALSE);
    assert_eq!(push(&mo, 3, 300), S_OK);

    let first = pull(&mo, &[]);
    assert_eq!(first.hr, S_OK);
    assert_eq!(first.time, 100);
    assert_eq!(first.bytes, frame(1));
    assert_eq!(first.status & incomplete, 0);
    assert_ne!(first.status & has_time, 0);
    assert_eq!(pull(&mo, &[]).hr, S_FALSE);

    assert_eq!(discontinuity(&mo), S_OK);

    let second = pull(&mo, &[]);
    assert_eq!(second.hr, S_OK);
    assert_eq!(second.time, 200);
    assert_eq!(second.bytes, frame(2));
    assert_ne!(second.status & incomplete, 0);

    let third = pull(&mo, &[]);
    assert_eq!(third.hr, S_OK);
    assert_eq!(third.time, 300);
    assert_eq!(third.bytes, frame(3));
    assert_eq!(third.status & incomplete, 0);

    assert_eq!(pull(&mo, &[]).hr, S_FALSE);
}

#[test]
fn flush_discards_queued_frames_and_accepts_input_again() {
    let mo = negotiated_dmo(2);
    assert_eq!(push(&mo, 1, 100), S_FALSE);
    assert_eq!(push(&mo, 2, 200), S_FALSE);

    assert_eq!(unsafe { (mo.vtable().Flush)(mo.as_raw()) }, S_OK);
    assert_eq!(discontinuity(&mo), S_OK);
    assert_eq!(pull(&mo, &[]).hr, S_FALSE);

    let mut flags = 0u32;
    let hr = unsafe { (mo.vtable().GetInputStatus)(mo.as_raw(), 0, &mut flags) };
    assert_eq!(hr, S_OK);
    assert_ne!(flags & DMO_INPUT_STATUSF_ACCEPT_DATA.0 as u32, 0);
}

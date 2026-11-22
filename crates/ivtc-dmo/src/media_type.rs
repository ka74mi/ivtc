//! DMO_MEDIA_TYPE と ivtc-core の VideoInfo の橋渡し。

use ivtc_core::video_info::{VideoInfo, VideoInfoKind};
use windows::Win32::Foundation::E_POINTER;
use windows::Win32::Media::DxMediaObjects::{
    DMO_E_TYPE_NOT_ACCEPTED, DMO_MEDIA_TYPE, MoInitMediaType,
};
use windows::core::{BOOL, GUID};

use crate::processor::DmoResult;

pub(crate) const MEDIATYPE_VIDEO: GUID = GUID::from_u128(0x73646976_0000_0010_8000_00aa00389b71);
pub(crate) const MEDIASUBTYPE_NV12: GUID = GUID::from_u128(0x3231564e_0000_0010_8000_00aa00389b71);
const FORMAT_VIDEOINFO: GUID = GUID::from_u128(0x05589f80_c356_11ce_bf01_00aa0055595a);
const FORMAT_VIDEOINFO2: GUID = GUID::from_u128(0xf72a76a0_eb0a_11d0_ace4_0000c0cc16ba);

/// format blockの異常に大きい値を弾く上限。
const MAX_FORMAT_BYTES: u32 = 64 * 1024;

/// 検証済みのNV12メディアタイプ。format blockはVecに深くコピーして保持する。
#[derive(Clone, PartialEq)]
pub(crate) struct NegotiatedType {
    pub(crate) info: VideoInfo,
    format: Vec<u8>,
}

impl NegotiatedType {
    /// # Safety
    /// `p` は null か、有効な DMO_MEDIA_TYPE を指すこと。
    pub(crate) unsafe fn from_raw(p: *const DMO_MEDIA_TYPE) -> DmoResult<Self> {
        if p.is_null() {
            return Err(E_POINTER);
        }
        let mt = unsafe { &*p };
        if mt.majortype != MEDIATYPE_VIDEO || mt.subtype != MEDIASUBTYPE_NV12 {
            return Err(DMO_E_TYPE_NOT_ACCEPTED);
        }
        let kind = if mt.formattype == FORMAT_VIDEOINFO2 {
            VideoInfoKind::V2
        } else if mt.formattype == FORMAT_VIDEOINFO {
            VideoInfoKind::V1
        } else {
            return Err(DMO_E_TYPE_NOT_ACCEPTED);
        };
        if mt.pbFormat.is_null() || mt.cbFormat == 0 || mt.cbFormat > MAX_FORMAT_BYTES {
            return Err(DMO_E_TYPE_NOT_ACCEPTED);
        }
        let format =
            unsafe { std::slice::from_raw_parts(mt.pbFormat, mt.cbFormat as usize) }.to_vec();
        match VideoInfo::parse(kind, &format) {
            Ok(info) => Ok(Self { info, format }),
            Err(e) => {
                trace!("type rejected: {e:?}");
                Err(DMO_E_TYPE_NOT_ACCEPTED)
            }
        }
    }

    pub(crate) fn frame_size(&self) -> usize {
        self.info.layout.frame_size()
    }

    pub(crate) fn describe(&self) -> String {
        let l = &self.info.layout;
        format!(
            "{:?} {}x{} stride={} planeH={} avgTime={} interlace={:#x}",
            self.info.kind,
            l.width,
            l.height,
            l.stride,
            l.plane_height,
            self.info.avg_time_per_frame,
            self.info.interlace_flags
        )
    }

    /// 呼び出し側(wrapper)が MoFreeMediaType で解放できる形で書き出す。
    ///
    /// # Safety
    /// `dest` は書き込み可能な DMO_MEDIA_TYPE を指すこと。
    pub(crate) unsafe fn write_to(&self, dest: *mut DMO_MEDIA_TYPE) -> DmoResult<()> {
        if dest.is_null() {
            return Err(E_POINTER);
        }
        unsafe { MoInitMediaType(dest, self.format.len() as u32) }.map_err(|e| e.code())?;
        let d = unsafe { &mut *dest };
        d.majortype = MEDIATYPE_VIDEO;
        d.subtype = MEDIASUBTYPE_NV12;
        d.bFixedSizeSamples = BOOL(1);
        d.bTemporalCompression = BOOL(0);
        d.lSampleSize = self.frame_size() as u32;
        d.formattype = match self.info.kind {
            VideoInfoKind::V1 => FORMAT_VIDEOINFO,
            VideoInfoKind::V2 => FORMAT_VIDEOINFO2,
        };
        unsafe {
            core::ptr::copy_nonoverlapping(self.format.as_ptr(), d.pbFormat, self.format.len())
        };
        Ok(())
    }
}

/// 入力側の「NV12ならなんでも」を表す部分型(format blockなし)。
///
/// # Safety
/// `dest` は書き込み可能な DMO_MEDIA_TYPE を指すこと。
pub(crate) unsafe fn write_partial_nv12(dest: *mut DMO_MEDIA_TYPE) -> DmoResult<()> {
    if dest.is_null() {
        return Err(E_POINTER);
    }
    unsafe { MoInitMediaType(dest, 0) }.map_err(|e| e.code())?;
    let d = unsafe { &mut *dest };
    d.majortype = MEDIATYPE_VIDEO;
    d.subtype = MEDIASUBTYPE_NV12;
    d.bFixedSizeSamples = BOOL(1);
    d.formattype = GUID::zeroed();
    Ok(())
}

//! VIDEOINFOHEADER / VIDEOINFOHEADER2 のバイト列(DirectShowのformat block)の解析。
//! Windows APIは使わず、リトルエンディアンのオフセットで読む。

use crate::nv12::{LayoutError, Nv12Layout};

/// BITMAPINFOHEADER.biCompression の 'NV12'。
pub const BI_NV12: u32 = 0x3231_564E;
pub const VIDEOINFOHEADER_SIZE: usize = 88;
pub const VIDEOINFOHEADER2_SIZE: usize = 112;

const OFF_RC_SOURCE: usize = 0;
const OFF_AVG_TIME_PER_FRAME: usize = 40;
const OFF_INTERLACE_FLAGS_V2: usize = 48;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoInfoKind {
    V1,
    V2,
}

impl VideoInfoKind {
    pub fn header_size(self) -> usize {
        match self {
            Self::V1 => VIDEOINFOHEADER_SIZE,
            Self::V2 => VIDEOINFOHEADER2_SIZE,
        }
    }

    fn bitmap_offset(self) -> usize {
        match self {
            Self::V1 => 48,
            Self::V2 => 72,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoInfoError {
    TooShort,
    NotNv12,
    BadBitmapHeader,
    Layout(LayoutError),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VideoInfo {
    pub kind: VideoInfoKind,
    pub layout: Nv12Layout,
    /// 100ns単位。0なら不明。
    pub avg_time_per_frame: i64,
    /// VIDEOINFOHEADER2.dwInterlaceFlags(V1では0)。
    pub interlace_flags: u32,
}

fn u32_at(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}
fn i32_at(b: &[u8], o: usize) -> i32 {
    i32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}

impl VideoInfo {
    pub fn parse(kind: VideoInfoKind, bytes: &[u8]) -> Result<Self, VideoInfoError> {
        if bytes.len() < kind.header_size() {
            return Err(VideoInfoError::TooShort);
        }

        let bmi = kind.bitmap_offset();
        let bi_size = u32_at(bytes, bmi);
        let bi_width = i32_at(bytes, bmi + 4);
        let bi_height = i32_at(bytes, bmi + 8);
        let bi_planes = u16::from_le_bytes([bytes[bmi + 12], bytes[bmi + 13]]);
        let bi_bit_count = u16::from_le_bytes([bytes[bmi + 14], bytes[bmi + 15]]);
        let bi_compression = u32_at(bytes, bmi + 16);

        if bi_compression != BI_NV12 {
            return Err(VideoInfoError::NotNv12);
        }
        if bi_size < 40 || bi_width <= 0 || bi_height == 0 || bi_planes != 1 || bi_bit_count != 12 {
            return Err(VideoInfoError::BadBitmapHeader);
        }

        // YUVでは biWidth がストライド(ピクセル=バイト)、|biHeight| がY平面の行数。
        // 実際の表示領域は rcSource があればそれを使う(原点オフセットは無視)。
        let stride = bi_width as u32;
        let plane_height = bi_height.unsigned_abs();
        let rc_w = i32_at(bytes, OFF_RC_SOURCE + 8) - i32_at(bytes, OFF_RC_SOURCE);
        let rc_h = i32_at(bytes, OFF_RC_SOURCE + 12) - i32_at(bytes, OFF_RC_SOURCE + 4);
        let (width, height) =
            if rc_w > 0 && rc_h > 0 && rc_w as u32 <= stride && rc_h as u32 <= plane_height {
                (rc_w as u32, rc_h as u32)
            } else {
                (stride, plane_height)
            };

        let layout =
            Nv12Layout::new(width, height, stride, plane_height).map_err(VideoInfoError::Layout)?;

        let avg_time_per_frame = i64::from_le_bytes(
            bytes[OFF_AVG_TIME_PER_FRAME..OFF_AVG_TIME_PER_FRAME + 8]
                .try_into()
                .unwrap(),
        );
        let interlace_flags = match kind {
            VideoInfoKind::V1 => 0,
            VideoInfoKind::V2 => u32_at(bytes, OFF_INTERLACE_FLAGS_V2),
        };

        Ok(Self {
            kind,
            layout,
            avg_time_per_frame,
            interlace_flags,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build(kind: VideoInfoKind, w: i32, h: i32, rc: Option<(i32, i32)>, fourcc: u32) -> Vec<u8> {
        let mut b = vec![0u8; kind.header_size()];
        if let Some((rw, rh)) = rc {
            b[8..12].copy_from_slice(&rw.to_le_bytes());
            b[12..16].copy_from_slice(&rh.to_le_bytes());
        }
        b[40..48].copy_from_slice(&333_667i64.to_le_bytes());
        if kind == VideoInfoKind::V2 {
            b[48..52].copy_from_slice(&1u32.to_le_bytes());
        }
        let o = kind.bitmap_offset();
        b[o..o + 4].copy_from_slice(&40u32.to_le_bytes());
        b[o + 4..o + 8].copy_from_slice(&w.to_le_bytes());
        b[o + 8..o + 12].copy_from_slice(&h.to_le_bytes());
        b[o + 12..o + 14].copy_from_slice(&1u16.to_le_bytes());
        b[o + 14..o + 16].copy_from_slice(&12u16.to_le_bytes());
        b[o + 16..o + 20].copy_from_slice(&fourcc.to_le_bytes());
        b
    }

    #[test]
    fn parses_v2_with_crop() {
        let b = build(VideoInfoKind::V2, 1536, 1088, Some((1440, 1080)), BI_NV12);
        let v = VideoInfo::parse(VideoInfoKind::V2, &b).unwrap();
        assert_eq!(v.layout, Nv12Layout::new(1440, 1080, 1536, 1088).unwrap());
        assert_eq!(v.avg_time_per_frame, 333_667);
        assert_eq!(v.interlace_flags, 1);
    }

    #[test]
    fn parses_v1_without_crop() {
        let b = build(VideoInfoKind::V1, 1920, -1080, None, BI_NV12);
        let v = VideoInfo::parse(VideoInfoKind::V1, &b).unwrap();
        assert_eq!(v.layout, Nv12Layout::packed(1920, 1080).unwrap());
        assert_eq!(v.interlace_flags, 0);
    }

    #[test]
    fn rejects_other_formats_and_short_blocks() {
        let b = build(VideoInfoKind::V2, 16, 16, None, 0x3231_5659);
        assert_eq!(
            VideoInfo::parse(VideoInfoKind::V2, &b),
            Err(VideoInfoError::NotNv12)
        );
        assert_eq!(
            VideoInfo::parse(VideoInfoKind::V2, &b[..50]),
            Err(VideoInfoError::TooShort)
        );
        let odd = build(VideoInfoKind::V1, 17, 16, None, BI_NV12);
        assert!(matches!(
            VideoInfo::parse(VideoInfoKind::V1, &odd),
            Err(VideoInfoError::Layout(LayoutError::OddSize))
        ));
    }
}

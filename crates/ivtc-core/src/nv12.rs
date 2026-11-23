#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Nv12Layout {
    pub width: u32,

    pub height: u32,

    pub stride: u32,

    pub plane_height: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayoutError {
    ZeroSize,
    OddSize,
    StrideTooSmall,
    PlaneTooShort,
    TooLarge,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CopyError {
    PictureMismatch,
    SourceTooShort,
    DestinationTooShort,
}

impl Nv12Layout {
    pub const MAX_DIMENSION: u32 = 16384;

    pub fn new(
        width: u32,
        height: u32,
        stride: u32,
        plane_height: u32,
    ) -> Result<Self, LayoutError> {
        if width == 0 || height == 0 {
            return Err(LayoutError::ZeroSize);
        }
        if width > Self::MAX_DIMENSION
            || plane_height > Self::MAX_DIMENSION
            || stride > Self::MAX_DIMENSION * 4
        {
            return Err(LayoutError::TooLarge);
        }
        if !width.is_multiple_of(2) || !height.is_multiple_of(2) || !plane_height.is_multiple_of(2)
        {
            return Err(LayoutError::OddSize);
        }
        if stride < width {
            return Err(LayoutError::StrideTooSmall);
        }
        if plane_height < height {
            return Err(LayoutError::PlaneTooShort);
        }
        Ok(Self {
            width,
            height,
            stride,
            plane_height,
        })
    }

    pub fn packed(width: u32, height: u32) -> Result<Self, LayoutError> {
        Self::new(width, height, width, height)
    }

    pub fn frame_size(&self) -> usize {
        self.stride as usize * self.plane_height as usize * 3 / 2
    }

    pub fn uv_offset(&self) -> usize {
        self.stride as usize * self.plane_height as usize
    }

    pub fn same_picture(&self, other: &Self) -> bool {
        self.width == other.width && self.height == other.height
    }
}

pub fn copy_frame(
    src: &[u8],
    src_layout: &Nv12Layout,
    dst: &mut [u8],
    dst_layout: &Nv12Layout,
) -> Result<(), CopyError> {
    if !src_layout.same_picture(dst_layout) {
        return Err(CopyError::PictureMismatch);
    }
    if src.len() < src_layout.frame_size() {
        return Err(CopyError::SourceTooShort);
    }
    if dst.len() < dst_layout.frame_size() {
        return Err(CopyError::DestinationTooShort);
    }

    if src_layout == dst_layout {
        let n = src_layout.frame_size();
        dst[..n].copy_from_slice(&src[..n]);
        return Ok(());
    }

    let width = src_layout.width as usize;
    let (ss, ds) = (src_layout.stride as usize, dst_layout.stride as usize);
    let (suv, duv) = (src_layout.uv_offset(), dst_layout.uv_offset());
    for y in 0..src_layout.height as usize {
        dst[y * ds..y * ds + width].copy_from_slice(&src[y * ss..y * ss + width]);
    }
    for y in 0..src_layout.height as usize / 2 {
        dst[duv + y * ds..duv + y * ds + width]
            .copy_from_slice(&src[suv + y * ss..suv + y * ss + width]);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_bad_layouts() {
        assert_eq!(Nv12Layout::packed(0, 4), Err(LayoutError::ZeroSize));
        assert_eq!(Nv12Layout::packed(5, 4), Err(LayoutError::OddSize));
        assert_eq!(
            Nv12Layout::new(8, 4, 6, 4),
            Err(LayoutError::StrideTooSmall)
        );
        assert_eq!(Nv12Layout::new(8, 4, 8, 2), Err(LayoutError::PlaneTooShort));
        assert_eq!(Nv12Layout::packed(100000, 4), Err(LayoutError::TooLarge));
    }

    #[test]
    fn frame_size_includes_padding() {
        let l = Nv12Layout::new(1440, 1080, 1536, 1088).unwrap();
        assert_eq!(l.frame_size(), 1536 * 1088 * 3 / 2);
        assert_eq!(l.uv_offset(), 1536 * 1088);
    }

    #[test]
    fn identical_layout_is_plain_copy() {
        let l = Nv12Layout::packed(4, 2).unwrap();
        let src: Vec<u8> = (0..12).collect();
        let mut dst = vec![0u8; 12];
        copy_frame(&src, &l, &mut dst, &l).unwrap();
        assert_eq!(src, dst);
    }

    #[test]
    fn converts_stride_for_both_planes() {
        let s = Nv12Layout::packed(4, 2).unwrap();
        let d = Nv12Layout::new(4, 2, 6, 2).unwrap();

        let src = vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12];
        let mut dst = vec![0xEEu8; d.frame_size()];
        copy_frame(&src, &s, &mut dst, &d).unwrap();
        assert_eq!(&dst[0..6], &[1, 2, 3, 4, 0xEE, 0xEE]);
        assert_eq!(&dst[6..12], &[5, 6, 7, 8, 0xEE, 0xEE]);
        assert_eq!(&dst[12..18], &[9, 10, 11, 12, 0xEE, 0xEE]);
    }

    #[test]
    fn reports_mismatch_and_short_buffers() {
        let a = Nv12Layout::packed(4, 2).unwrap();
        let b = Nv12Layout::packed(6, 2).unwrap();
        let mut dst = vec![0u8; 64];
        assert_eq!(
            copy_frame(&[0; 64], &a, &mut dst, &b),
            Err(CopyError::PictureMismatch)
        );
        assert_eq!(
            copy_frame(&[0; 3], &a, &mut dst, &a),
            Err(CopyError::SourceTooShort)
        );
        let mut small = [0u8; 3];
        assert_eq!(
            copy_frame(&[0; 64], &a, &mut small, &a),
            Err(CopyError::DestinationTooShort)
        );
    }
}

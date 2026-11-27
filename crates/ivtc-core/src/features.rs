use crate::nv12::Nv12Layout;

pub const BLOCK_WIDTH: usize = 16;
pub const BLOCK_HEIGHT: usize = 32;
const WINDOW_ROWS: usize = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldOrder {
    TopFirst,
    BottomFirst,
}

impl FieldOrder {
    fn first_parity(self) -> usize {
        match self {
            Self::TopFirst => 0,
            Self::BottomFirst => 1,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeatureError {
    PictureTooSmall,
    FrameTooShort,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CombScore {
    pub mean: f64,
    pub max_block: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FrameFeatures {
    pub comb_c: CombScore,
    pub comb_p: Option<CombScore>,
    pub comb_n: Option<CombScore>,
    pub sad_fs: Option<f64>,
    pub sad_ss: Option<f64>,
    pub sad_po: Option<f64>,
    pub sad_co: f64,
    pub sad_frame: Option<f64>,
    pub mean_luma: f64,
}

#[derive(Clone, Copy)]
struct Luma<'a> {
    data: &'a [u8],
    stride: usize,
    width: usize,
    height: usize,
}

impl<'a> Luma<'a> {
    fn new(data: &'a [u8], layout: &Nv12Layout) -> Self {
        Self {
            data,
            stride: layout.stride as usize,
            width: layout.width as usize,
            height: layout.height as usize,
        }
    }

    fn row(&self, y: usize) -> &'a [u8] {
        let start = y * self.stride;
        &self.data[start..start + self.width]
    }
}

fn cored_diff(a: u8, b: u8, coring: i32) -> i32 {
    let d = i32::from(b) - i32::from(a);
    if d.abs() <= coring { 0 } else { d }
}

fn reversals(p: [u8; WINDOW_ROWS], coring: i32) -> u64 {
    let d0 = cored_diff(p[0], p[1], coring);
    let d1 = cored_diff(p[1], p[2], coring);
    let d2 = cored_diff(p[2], p[3], coring);
    u64::from(d0 * d1 < 0) + u64::from(d1 * d2 < 0)
}

fn comb_score(top: Luma, bottom: Luma, coring: i32) -> CombScore {
    let width = top.width;
    let height = top.height;
    let windows = height - WINDOW_ROWS + 1;
    let blocks_x = width.div_ceil(BLOCK_WIDTH);
    let blocks_y = windows.div_ceil(BLOCK_HEIGHT);
    let mut blocks = vec![0u64; blocks_x * blocks_y];
    let mut total = 0u64;

    let pick = |y: usize| {
        if y.is_multiple_of(2) {
            top.row(y)
        } else {
            bottom.row(y)
        }
    };

    for y in 0..windows {
        let rows = [pick(y), pick(y + 1), pick(y + 2), pick(y + 3)];
        let block_row = (y / BLOCK_HEIGHT) * blocks_x;
        for x in 0..width {
            let count = reversals([rows[0][x], rows[1][x], rows[2][x], rows[3][x]], coring);
            if count != 0 {
                total += count;
                blocks[block_row + x / BLOCK_WIDTH] += count;
            }
        }
    }

    let max_block = blocks.iter().copied().max().unwrap_or(0);
    CombScore {
        mean: total as f64 / (width * height) as f64,
        max_block: max_block as f64 / (BLOCK_WIDTH * BLOCK_HEIGHT) as f64,
    }
}

fn field_sad(a: Luma, parity_a: usize, b: Luma, parity_b: usize) -> f64 {
    let rows = a.height / 2;
    let mut sum = 0u64;
    for k in 0..rows {
        let ra = a.row(2 * k + parity_a);
        let rb = b.row(2 * k + parity_b);
        sum += ra
            .iter()
            .zip(rb)
            .map(|(&x, &y)| u64::from(x.abs_diff(y)))
            .sum::<u64>();
    }
    sum as f64 / (rows * a.width) as f64
}

fn frame_sad(a: Luma, b: Luma) -> f64 {
    let mut sum = 0u64;
    for y in 0..a.height {
        sum += a
            .row(y)
            .iter()
            .zip(b.row(y))
            .map(|(&x, &z)| u64::from(x.abs_diff(z)))
            .sum::<u64>();
    }
    sum as f64 / (a.height * a.width) as f64
}

fn mean_luma(a: Luma) -> f64 {
    let mut sum = 0u64;
    for y in 0..a.height {
        sum += a.row(y).iter().map(|&x| u64::from(x)).sum::<u64>();
    }
    sum as f64 / (a.height * a.width) as f64
}

fn arrange<'a>(first_parity: usize, anchor: Luma<'a>, other: Luma<'a>) -> (Luma<'a>, Luma<'a>) {
    if first_parity == 0 {
        (anchor, other)
    } else {
        (other, anchor)
    }
}

pub fn analyze_frame(
    layout: &Nv12Layout,
    order: FieldOrder,
    coring: u8,
    prev: Option<&[u8]>,
    cur: &[u8],
    next: Option<&[u8]>,
) -> Result<FrameFeatures, FeatureError> {
    if (layout.height as usize) < WINDOW_ROWS {
        return Err(FeatureError::PictureTooSmall);
    }
    let size = layout.frame_size();
    if cur.len() < size
        || prev.is_some_and(|p| p.len() < size)
        || next.is_some_and(|n| n.len() < size)
    {
        return Err(FeatureError::FrameTooShort);
    }

    let coring = i32::from(coring);
    let first = order.first_parity();
    let second = 1 - first;
    let cur = Luma::new(cur, layout);
    let prev = prev.map(|d| Luma::new(d, layout));
    let next = next.map(|d| Luma::new(d, layout));

    let candidate = |other: Option<Luma>| {
        other.map(|o| {
            let (top, bottom) = arrange(first, cur, o);
            comb_score(top, bottom, coring)
        })
    };

    Ok(FrameFeatures {
        comb_c: comb_score(cur, cur, coring),
        comb_p: candidate(prev),
        comb_n: candidate(next),
        sad_fs: prev.map(|p| field_sad(cur, first, p, first)),
        sad_ss: prev.map(|p| field_sad(cur, second, p, second)),
        sad_po: prev.map(|p| field_sad(cur, first, p, second)),
        sad_co: field_sad(cur, first, cur, second),
        sad_frame: prev.map(|p| frame_sad(cur, p)),
        mean_luma: mean_luma(cur),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const W: u32 = 32;
    const H: u32 = 64;

    fn layout() -> Nv12Layout {
        Nv12Layout::packed(W, H).unwrap()
    }

    fn frame(top: u8, bottom: u8) -> Vec<u8> {
        let l = layout();
        let mut buf = vec![128u8; l.frame_size()];
        for y in 0..H as usize {
            let v = if y % 2 == 0 { top } else { bottom };
            buf[y * W as usize..(y + 1) * W as usize].fill(v);
        }
        buf
    }

    fn assert_close(actual: f64, expected: f64) {
        assert!(
            (actual - expected).abs() < 1e-9,
            "actual={actual} expected={expected}"
        );
    }

    #[test]
    fn static_flat_frames_have_no_comb_and_no_difference() {
        let f = frame(100, 100);
        let r = analyze_frame(&layout(), FieldOrder::TopFirst, 4, Some(&f), &f, Some(&f)).unwrap();
        assert_eq!(r.comb_c.mean, 0.0);
        assert_eq!(r.comb_c.max_block, 0.0);
        assert_eq!(r.comb_p.unwrap().mean, 0.0);
        assert_eq!(r.comb_n.unwrap().mean, 0.0);
        assert_eq!(r.sad_fs, Some(0.0));
        assert_eq!(r.sad_ss, Some(0.0));
        assert_eq!(r.sad_po, Some(0.0));
        assert_eq!(r.sad_co, 0.0);
        assert_eq!(r.sad_frame, Some(0.0));
        assert_eq!(r.mean_luma, 100.0);
    }

    #[test]
    fn fully_combed_frame_counts_two_reversals_per_window_pixel() {
        let cur = frame(200, 50);
        let r = analyze_frame(&layout(), FieldOrder::TopFirst, 4, None, &cur, None).unwrap();
        assert_close(r.comb_c.mean, 2.0 * f64::from(H - 3) / f64::from(H));
        assert_close(r.comb_c.max_block, 2.0);
    }

    #[test]
    fn coring_suppresses_small_differences() {
        let cur = frame(200, 50);
        let r = analyze_frame(&layout(), FieldOrder::TopFirst, 150, None, &cur, None).unwrap();
        assert_eq!(r.comb_c.mean, 0.0);
        assert_eq!(r.comb_c.max_block, 0.0);
    }

    #[test]
    fn missing_neighbours_yield_none() {
        let cur = frame(10, 20);
        let r = analyze_frame(&layout(), FieldOrder::TopFirst, 4, None, &cur, None).unwrap();
        assert_eq!(r.comb_p, None);
        assert_eq!(r.comb_n, None);
        assert_eq!(r.sad_fs, None);
        assert_eq!(r.sad_ss, None);
        assert_eq!(r.sad_po, None);
        assert_eq!(r.sad_frame, None);
        assert_close(r.sad_co, 10.0);
    }

    #[test]
    fn top_first_candidates_pair_current_top_with_neighbour_bottom() {
        let prev = frame(0, 200);
        let cur = frame(200, 50);
        let next = frame(0, 50);
        let r = analyze_frame(
            &layout(),
            FieldOrder::TopFirst,
            4,
            Some(&prev),
            &cur,
            Some(&next),
        )
        .unwrap();
        assert!(r.comb_c.mean > 0.0);
        assert_eq!(r.comb_p.unwrap().mean, 0.0);
        assert!(r.comb_n.unwrap().mean > 0.0);
    }

    #[test]
    fn bottom_first_candidates_pair_current_bottom_with_neighbour_top() {
        let prev = frame(50, 0);
        let cur = frame(200, 50);
        let next = frame(200, 0);
        let r = analyze_frame(
            &layout(),
            FieldOrder::BottomFirst,
            4,
            Some(&prev),
            &cur,
            Some(&next),
        )
        .unwrap();
        assert!(r.comb_c.mean > 0.0);
        assert_eq!(r.comb_p.unwrap().mean, 0.0);
        assert!(r.comb_n.unwrap().mean > 0.0);

        let swapped = analyze_frame(
            &layout(),
            FieldOrder::TopFirst,
            4,
            Some(&prev),
            &cur,
            Some(&next),
        )
        .unwrap();
        assert!(swapped.comb_p.unwrap().mean > 0.0);
    }

    #[test]
    fn sad_metrics_follow_field_order() {
        let prev = frame(10, 25);
        let cur = frame(20, 30);

        let tff =
            analyze_frame(&layout(), FieldOrder::TopFirst, 4, Some(&prev), &cur, None).unwrap();
        assert_close(tff.sad_fs.unwrap(), 10.0);
        assert_close(tff.sad_ss.unwrap(), 5.0);
        assert_close(tff.sad_po.unwrap(), 5.0);
        assert_close(tff.sad_co, 10.0);
        assert_close(tff.sad_frame.unwrap(), 7.5);
        assert_close(tff.mean_luma, 25.0);

        let bff = analyze_frame(
            &layout(),
            FieldOrder::BottomFirst,
            4,
            Some(&prev),
            &cur,
            None,
        )
        .unwrap();
        assert_close(bff.sad_fs.unwrap(), 5.0);
        assert_close(bff.sad_ss.unwrap(), 10.0);
        assert_close(bff.sad_po.unwrap(), 20.0);
        assert_close(bff.sad_co, 10.0);
    }

    #[test]
    fn rejects_short_buffers_and_tiny_pictures() {
        let cur = frame(1, 2);
        assert_eq!(
            analyze_frame(&layout(), FieldOrder::TopFirst, 4, None, &cur[..10], None),
            Err(FeatureError::FrameTooShort)
        );
        assert_eq!(
            analyze_frame(
                &layout(),
                FieldOrder::TopFirst,
                4,
                Some(&cur[..10]),
                &cur,
                None
            ),
            Err(FeatureError::FrameTooShort)
        );
        let tiny = Nv12Layout::packed(8, 2).unwrap();
        let buf = vec![0u8; tiny.frame_size()];
        assert_eq!(
            analyze_frame(&tiny, FieldOrder::TopFirst, 4, None, &buf, None),
            Err(FeatureError::PictureTooSmall)
        );
    }

    #[test]
    fn honours_stride_padding() {
        let padded = Nv12Layout::new(W, H, W + 8, H + 2).unwrap();
        let mut buf = vec![255u8; padded.frame_size()];
        for y in 0..H as usize {
            let v = if y % 2 == 0 { 200 } else { 50 };
            let start = y * padded.stride as usize;
            buf[start..start + W as usize].fill(v);
        }
        let r = analyze_frame(&padded, FieldOrder::TopFirst, 4, None, &buf, None).unwrap();
        assert_close(r.comb_c.mean, 2.0 * f64::from(H - 3) / f64::from(H));
        assert_close(r.sad_co, 150.0);
        assert_close(r.mean_luma, 125.0);
    }
}

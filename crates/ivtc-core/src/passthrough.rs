//! パススルー処理の中核(COM/Windows非依存)。
//!
//! - フレームは「入力時のレイアウト」と一緒にキューへ積む。出力時にそのレイアウトから
//!   現在の出力レイアウトへ変換コピーするので、入力/出力の型(stride)が途中で変わっても
//!   キュー内のフレームを捨てる必要がない。
//! - 出力型の設定はキューに触らない。(DMO Wrapperは出力型を頻繁に再設定してくる)
//! - 入力の表示サイズが変わったときだけ、出力型を無効化してキューを捨てる。

use crate::delay_queue::DelayQueue;
use crate::nv12::{Nv12Layout, copy_frame};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SampleMeta {
    pub time: Option<i64>,
    pub length: Option<i64>,
    pub discontinuity: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutputMeta {
    pub sample: SampleMeta,
    /// まだ出力できるフレームが残っている。
    pub more: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PassError {
    TypeNotSet,
    PictureMismatch,
    NotAccepting,
    ShortInput { got: usize, need: usize },
    OutputTooSmall { got: usize, need: usize },
    CopyFailed,
}

struct Frame {
    data: Vec<u8>,
    layout: Nv12Layout,
    meta: SampleMeta,
}

const POOL_MAX: usize = 8;

pub struct Passthrough {
    input: Option<Nv12Layout>,
    output: Option<Nv12Layout>,
    queue: DelayQueue<Frame>,
    pool: Vec<Vec<u8>>,
    frames_in: u64,
    frames_out: u64,
    frames_dropped: u64,
}

impl Passthrough {
    pub fn new(delay: usize) -> Self {
        Self {
            input: None,
            output: None,
            queue: DelayQueue::new(delay),
            pool: Vec::new(),
            frames_in: 0,
            frames_out: 0,
            frames_dropped: 0,
        }
    }

    pub fn delay(&self) -> usize {
        self.queue.delay()
    }

    pub fn queued(&self) -> usize {
        self.queue.len()
    }

    /// (入力数, 出力数, 破棄数)
    pub fn counters(&self) -> (u64, u64, u64) {
        (self.frames_in, self.frames_out, self.frames_dropped)
    }

    pub fn input_layout(&self) -> Option<&Nv12Layout> {
        self.input.as_ref()
    }

    pub fn output_layout(&self) -> Option<&Nv12Layout> {
        self.output.as_ref()
    }

    // ---- 型 ------------------------------------------------------------

    /// 入力レイアウトを設定する。出力型を無効化した(=呼び出し側も出力型を捨てるべき)ときtrue。
    /// 表示サイズが変わらない限りキューは捨てない。
    pub fn set_input_layout(&mut self, layout: Nv12Layout) -> bool {
        let picture_changed = self
            .input
            .as_ref()
            .is_some_and(|old| !old.same_picture(&layout));
        let mut output_cleared = false;
        if picture_changed {
            self.discard_queued();
            self.output = None;
            output_cleared = true;
        } else if let Some(out) = &self.output
            && !out.same_picture(&layout)
        {
            self.output = None;
            output_cleared = true;
        }
        self.input = Some(layout);
        output_cleared
    }

    pub fn clear_input(&mut self) {
        self.discard_queued();
        self.input = None;
        self.output = None;
    }

    /// 出力レイアウトとして受け入れ可能か(設定はしない)。
    pub fn check_output_layout(&self, layout: &Nv12Layout) -> Result<(), PassError> {
        let input = self.input.as_ref().ok_or(PassError::TypeNotSet)?;
        if input.same_picture(layout) {
            Ok(())
        } else {
            Err(PassError::PictureMismatch)
        }
    }

    /// 出力レイアウトを設定する。キューには触らない。
    pub fn set_output_layout(&mut self, layout: Nv12Layout) -> Result<(), PassError> {
        self.check_output_layout(&layout)?;
        self.output = Some(layout);
        Ok(())
    }

    pub fn clear_output(&mut self) {
        self.output = None;
    }

    fn both(&self) -> Result<(&Nv12Layout, &Nv12Layout), PassError> {
        match (&self.input, &self.output) {
            (Some(i), Some(o)) => Ok((i, o)),
            _ => Err(PassError::TypeNotSet),
        }
    }

    // ---- ストリーミング --------------------------------------------------

    pub fn accepts_input(&self) -> Result<bool, PassError> {
        self.both()?;
        Ok(self.queue.accepts_input())
    }

    pub fn has_ready(&self) -> bool {
        self.queue.has_ready()
    }

    pub fn output_frame_size(&self) -> Result<usize, PassError> {
        Ok(self.both()?.1.frame_size())
    }

    /// 入力を1枚受ける。戻り値は「いま出力できるフレームがあるか」。
    pub fn push(&mut self, data: &[u8], meta: SampleMeta) -> Result<bool, PassError> {
        let (input, _) = self.both()?;
        let layout = *input;
        if !self.queue.accepts_input() {
            return Err(PassError::NotAccepting);
        }
        if data.is_empty() {
            return Ok(self.queue.has_ready());
        }
        let need = layout.frame_size();
        if data.len() < need {
            return Err(PassError::ShortInput {
                got: data.len(),
                need,
            });
        }

        let mut buf = self.pool.pop().unwrap_or_default();
        buf.clear();
        buf.extend_from_slice(&data[..need]);
        self.queue.push(Frame {
            data: buf,
            layout,
            meta,
        });
        self.frames_in += 1;
        Ok(self.queue.has_ready())
    }

    /// 出力できるフレームを1枚 `dst` に書く。`dst` はちょうど出力フレーム分の長さ以上。
    pub fn pop_into(&mut self, dst: &mut [u8]) -> Result<Option<OutputMeta>, PassError> {
        let output = *self.both()?.1;
        if !self.queue.has_ready() {
            return Ok(None);
        }
        let need = output.frame_size();
        if dst.len() < need {
            return Err(PassError::OutputTooSmall {
                got: dst.len(),
                need,
            });
        }
        let frame = self.queue.pop_ready().expect("has_ready() was true");
        let copied = copy_frame(&frame.data, &frame.layout, &mut dst[..need], &output);
        let out = OutputMeta {
            sample: frame.meta,
            more: self.queue.has_ready(),
        };
        self.recycle(frame.data);
        copied.map_err(|_| PassError::CopyFailed)?;
        self.frames_out += 1;
        Ok(Some(out))
    }

    /// 出力バッファが無いときに1枚捨てる。
    pub fn discard_ready(&mut self) {
        if let Some(frame) = self.queue.pop_ready() {
            self.frames_dropped += 1;
            self.recycle(frame.data);
        }
    }

    /// EOS/不連続: 残りをすべて吐き出せる状態にする。
    pub fn begin_drain(&mut self) -> Result<(), PassError> {
        self.both()?;
        self.queue.begin_drain();
        Ok(())
    }

    /// シーク等: 保持中のフレームを捨てる。捨てた枚数を返す。
    pub fn flush(&mut self) -> usize {
        self.discard_queued()
    }

    pub fn free_resources(&mut self) {
        self.discard_queued();
        self.pool.clear();
    }

    fn discard_queued(&mut self) -> usize {
        let frames = self.queue.clear();
        let n = frames.len();
        self.frames_dropped += n as u64;
        for f in frames {
            self.recycle(f.data);
        }
        n
    }

    fn recycle(&mut self, buf: Vec<u8>) {
        if self.pool.len() < POOL_MAX {
            self.pool.push(buf);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout(stride: u32) -> Nv12Layout {
        Nv12Layout::new(4, 2, stride, 2).unwrap()
    }

    fn frame(fill: u8, l: &Nv12Layout) -> Vec<u8> {
        vec![fill; l.frame_size()]
    }

    fn meta(t: i64) -> SampleMeta {
        SampleMeta {
            time: Some(t),
            length: Some(10),
            discontinuity: false,
        }
    }

    fn ready(delay: usize, input: Nv12Layout, output: Nv12Layout) -> Passthrough {
        let mut p = Passthrough::new(delay);
        p.set_input_layout(input);
        p.set_output_layout(output).unwrap();
        p
    }

    /// 回帰テスト: DMO Wrapper は入力直後に出力型を(同じ型でも)再設定してくる。
    /// これでキューが空になってはいけない。
    #[test]
    fn resetting_output_layout_keeps_queued_frames() {
        let (i, o) = (layout(4), layout(6));
        let mut p = ready(0, i, o);
        assert!(p.push(&frame(7, &i), meta(1)).unwrap());

        p.set_output_layout(o).unwrap();
        p.set_output_layout(o).unwrap();

        let mut dst = vec![0u8; o.frame_size()];
        let out = p.pop_into(&mut dst).unwrap().expect("frame must survive");
        assert_eq!(out.sample.time, Some(1));
        assert_eq!(&dst[..4], &[7, 7, 7, 7]);
        assert_eq!(p.counters(), (1, 1, 0));
    }

    #[test]
    fn output_stride_may_change_between_push_and_pop() {
        let i = layout(4);
        let mut p = ready(0, i, layout(4));
        p.push(&frame(9, &i), meta(1)).unwrap();
        let o2 = layout(8);
        p.set_output_layout(o2).unwrap();
        let mut dst = vec![0u8; o2.frame_size()];
        assert!(p.pop_into(&mut dst).unwrap().is_some());
        assert_eq!(&dst[..8], &[9, 9, 9, 9, 0, 0, 0, 0]);
    }

    #[test]
    fn input_stride_change_keeps_queue_and_uses_each_frames_layout() {
        let (a, b) = (layout(4), layout(6));
        let o = layout(4);
        let mut p = ready(1, a, o);
        p.push(&frame(1, &a), meta(1)).unwrap();

        // 同じ表示サイズでstrideだけ変わる: 出力型もキューも維持される
        assert!(!p.set_input_layout(b));
        p.push(&frame(2, &b), meta(2)).unwrap();

        let mut dst = vec![0u8; o.frame_size()];
        assert_eq!(p.pop_into(&mut dst).unwrap().unwrap().sample.time, Some(1));
        assert_eq!(&dst[..4], &[1, 1, 1, 1]);
        p.begin_drain().unwrap();
        assert_eq!(p.pop_into(&mut dst).unwrap().unwrap().sample.time, Some(2));
        assert_eq!(&dst[..4], &[2, 2, 2, 2]);
    }

    #[test]
    fn picture_change_clears_output_and_queue() {
        let a = layout(4);
        let mut p = ready(1, a, a);
        p.push(&frame(1, &a), meta(1)).unwrap();
        let bigger = Nv12Layout::packed(6, 2).unwrap();
        assert!(p.set_input_layout(bigger));
        assert_eq!(p.queued(), 0);
        assert!(p.output_layout().is_none());
        assert_eq!(p.counters().2, 1);
    }

    #[test]
    fn output_must_match_input_picture() {
        let a = layout(4);
        let mut p = Passthrough::new(0);
        assert_eq!(p.set_output_layout(a), Err(PassError::TypeNotSet));
        p.set_input_layout(a);
        let other = Nv12Layout::packed(6, 2).unwrap();
        assert_eq!(p.set_output_layout(other), Err(PassError::PictureMismatch));
    }

    #[test]
    fn delay_and_drain_emit_everything_in_order() {
        let a = layout(4);
        let mut p = ready(2, a, a);
        let mut dst = vec![0u8; a.frame_size()];
        for t in 1..=2 {
            assert!(!p.push(&frame(t as u8, &a), meta(t)).unwrap());
        }
        p.begin_drain().unwrap();
        let t: Vec<_> = std::iter::from_fn(|| {
            p.pop_into(&mut dst)
                .unwrap()
                .map(|o| o.sample.time.unwrap())
        })
        .collect();
        assert_eq!(t, vec![1, 2]);
        assert!(p.accepts_input().unwrap());
    }

    #[test]
    fn flush_discards_and_counts() {
        let a = layout(4);
        let mut p = ready(2, a, a);
        p.push(&frame(1, &a), meta(1)).unwrap();
        assert_eq!(p.flush(), 1);
        assert_eq!(p.counters(), (1, 0, 1));
    }

    #[test]
    fn rejects_short_input_and_full_queue() {
        let a = layout(4);
        let mut p = ready(0, a, a);
        assert!(matches!(
            p.push(&[0; 3], meta(1)),
            Err(PassError::ShortInput { .. })
        ));
        p.push(&frame(1, &a), meta(1)).unwrap();
        assert_eq!(p.push(&frame(2, &a), meta(2)), Err(PassError::NotAccepting));
    }

    #[test]
    fn small_output_buffer_is_reported_without_losing_the_frame() {
        let a = layout(4);
        let mut p = ready(0, a, a);
        p.push(&frame(1, &a), meta(1)).unwrap();
        let mut small = [0u8; 3];
        assert!(matches!(
            p.pop_into(&mut small),
            Err(PassError::OutputTooSmall { .. })
        ));
        let mut dst = vec![0u8; a.frame_size()];
        assert!(p.pop_into(&mut dst).unwrap().is_some());
    }
}

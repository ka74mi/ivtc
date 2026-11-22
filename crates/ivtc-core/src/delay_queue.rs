//! N枚の先読み(遅延)を持つフレームキュー。
//!
//! DMOの規約: ストリーミング中は「入力を受け付ける」か「出力を出せる」の
//! どちらか(または両方)が常に成り立たなければならない。
//! - `accepts_input()`: 入力を受けられる(保持数が delay 以下で、吐き出し中でない)
//! - `has_ready()`   : 出力できる(保持数が delay を超えた、または吐き出し中で空でない)
//!
//! EOS(Discontinuity)では `begin_drain()` で残りをすべて吐き出す。
//! 吐き出しが終わる(空になる)と自動で通常状態に戻る。

use std::collections::VecDeque;

#[derive(Debug)]
pub struct DelayQueue<T> {
    items: VecDeque<T>,
    delay: usize,
    draining: bool,
}

impl<T> DelayQueue<T> {
    pub fn new(delay: usize) -> Self {
        Self {
            items: VecDeque::new(),
            delay,
            draining: false,
        }
    }

    pub fn delay(&self) -> usize {
        self.delay
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn is_draining(&self) -> bool {
        self.draining
    }

    pub fn accepts_input(&self) -> bool {
        !self.draining && self.items.len() <= self.delay
    }

    pub fn has_ready(&self) -> bool {
        self.items.len() > self.delay || (self.draining && !self.items.is_empty())
    }

    /// 呼び出し側は先に `accepts_input()` を確認すること。
    pub fn push(&mut self, item: T) {
        self.items.push_back(item);
    }

    pub fn pop_ready(&mut self) -> Option<T> {
        if !self.has_ready() {
            return None;
        }
        let item = self.items.pop_front();
        if self.draining && self.items.is_empty() {
            self.draining = false;
        }
        item
    }

    /// EOS/不連続: 残りをすべて出力できる状態にする。
    pub fn begin_drain(&mut self) {
        self.draining = !self.items.is_empty();
    }

    /// シークなど: 保持しているフレームを捨てる。
    pub fn clear(&mut self) -> Vec<T> {
        self.draining = false;
        self.items.drain(..).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delay_zero_passes_each_item_through() {
        let mut q = DelayQueue::new(0);
        assert!(q.accepts_input());
        q.push(1);
        assert!(q.has_ready() && !q.accepts_input());
        assert_eq!(q.pop_ready(), Some(1));
        assert!(q.accepts_input() && !q.has_ready());
    }

    #[test]
    fn delay_two_holds_back_then_streams() {
        let mut q = DelayQueue::new(2);
        q.push(1);
        q.push(2);
        assert!(!q.has_ready() && q.accepts_input());
        q.push(3);
        assert!(q.has_ready() && !q.accepts_input());
        assert_eq!(q.pop_ready(), Some(1));
        assert!(q.accepts_input());
    }

    #[test]
    fn drain_emits_everything_in_order_then_resets() {
        let mut q = DelayQueue::new(3);
        for i in 1..=3 {
            q.push(i);
        }
        assert!(!q.has_ready());
        q.begin_drain();
        assert!(!q.accepts_input());
        assert_eq!(
            (q.pop_ready(), q.pop_ready(), q.pop_ready(), q.pop_ready()),
            (Some(1), Some(2), Some(3), None)
        );
        assert!(!q.is_draining() && q.accepts_input());
    }

    #[test]
    fn drain_on_empty_queue_is_a_no_op() {
        let mut q: DelayQueue<u8> = DelayQueue::new(1);
        q.begin_drain();
        assert!(q.accepts_input() && !q.is_draining());
    }

    #[test]
    fn clear_discards_and_resets() {
        let mut q = DelayQueue::new(2);
        q.push(1);
        q.push(2);
        q.begin_drain();
        assert_eq!(q.clear(), vec![1, 2]);
        assert!(q.is_empty() && q.accepts_input() && !q.has_ready());
    }
}

//! IVTC のコア。DirectShow / Windows API には依存しない(Linuxでも `cargo test` できる)。
//!
//! 現在はパススルーDMOに必要な部分だけ:
//! - [`nv12`]        : NV12フレームのレイアウトとストライド変換コピー
//! - [`video_info`]  : VIDEOINFOHEADER / VIDEOINFOHEADER2 のバイト列の解析
//! - [`delay_queue`] : 先読み・EOS吐き出し・Flushを扱う遅延キュー
//! - [`passthrough`] : 型(stride)が途中で変わっても安全なパススルー処理の中核
//!
//! 解析・ケイデンス検出・Weave は今後このクレートに追加する。

pub mod delay_queue;
pub mod nv12;
pub mod passthrough;
pub mod video_info;

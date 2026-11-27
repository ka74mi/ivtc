use std::io::{self, Read, Write};

use ivtc_core::features::{FrameFeatures, analyze_frame};

use crate::args::Config;

pub const CSV_HEADER: &str = "frame,comb_c_mean,comb_c_max,comb_p_mean,comb_p_max,comb_n_mean,comb_n_max,sad_fs,sad_ss,sad_po,sad_co,sad_frame,mean_luma";

fn value(v: Option<f64>) -> String {
    v.map(|x| format!("{x:.6}")).unwrap_or_default()
}

pub fn csv_row(index: u64, f: &FrameFeatures) -> String {
    let fields = [
        index.to_string(),
        value(Some(f.comb_c.mean)),
        value(Some(f.comb_c.max_block)),
        value(f.comb_p.map(|c| c.mean)),
        value(f.comb_p.map(|c| c.max_block)),
        value(f.comb_n.map(|c| c.mean)),
        value(f.comb_n.map(|c| c.max_block)),
        value(f.sad_fs),
        value(f.sad_ss),
        value(f.sad_po),
        value(Some(f.sad_co)),
        value(f.sad_frame),
        value(Some(f.mean_luma)),
    ];
    fields.join(",")
}

fn read_frame<R: Read>(reader: &mut R, buf: &mut [u8]) -> io::Result<bool> {
    let mut filled = 0;
    while filled < buf.len() {
        match reader.read(&mut buf[filled..]) {
            Ok(0) => break,
            Ok(n) => filled += n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    if filled == buf.len() {
        Ok(true)
    } else if filled == 0 {
        Ok(false)
    } else {
        Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            format!(
                "最後のフレームが途中で終わっています ({filled} / {} バイト)",
                buf.len()
            ),
        ))
    }
}

pub fn run<R: Read, W: Write>(
    mut reader: R,
    writer: &mut W,
    config: &Config,
) -> Result<u64, String> {
    let size = config.layout.frame_size();
    let read_error = |e: io::Error| format!("入力の読み込みに失敗しました: {e}");
    let write_error = |e: io::Error| format!("出力に失敗しました: {e}");

    let mut cur = vec![0u8; size];
    if !read_frame(&mut reader, &mut cur).map_err(read_error)? {
        return Err("入力にフレームがありません".to_string());
    }
    let mut prev: Option<Vec<u8>> = None;
    let mut next = vec![0u8; size];
    let mut index = 0u64;

    writeln!(writer, "{CSV_HEADER}").map_err(write_error)?;

    loop {
        let has_next = read_frame(&mut reader, &mut next).map_err(read_error)?;
        let features = analyze_frame(
            &config.layout,
            config.order,
            config.coring,
            prev.as_deref(),
            &cur,
            has_next.then_some(next.as_slice()),
        )
        .map_err(|e| format!("特徴量の計算に失敗しました: {e:?}"))?;
        writeln!(writer, "{}", csv_row(index, &features)).map_err(write_error)?;
        index += 1;

        if !has_next {
            return Ok(index);
        }
        let recycled = prev.take().unwrap_or_else(|| vec![0u8; size]);
        prev = Some(std::mem::replace(
            &mut cur,
            std::mem::replace(&mut next, recycled),
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ivtc_core::features::FieldOrder;
    use ivtc_core::nv12::Nv12Layout;

    fn config() -> Config {
        Config {
            layout: Nv12Layout::packed(8, 4).unwrap(),
            order: FieldOrder::TopFirst,
            coring: 4,
        }
    }

    fn frame(top: u8, bottom: u8) -> Vec<u8> {
        let mut buf = vec![128u8; config().layout.frame_size()];
        for y in 0..4usize {
            buf[y * 8..(y + 1) * 8].fill(if y % 2 == 0 { top } else { bottom });
        }
        buf
    }

    fn run_on(input: &[u8]) -> (Result<u64, String>, Vec<String>) {
        let mut out = Vec::new();
        let result = run(input, &mut out, &config());
        let lines = String::from_utf8(out)
            .unwrap()
            .lines()
            .map(str::to_string)
            .collect();
        (result, lines)
    }

    #[test]
    fn writes_header_and_one_row_per_frame_with_blank_edges() {
        let input: Vec<u8> = [frame(10, 20), frame(30, 40), frame(50, 60)].concat();
        let (result, lines) = run_on(&input);
        assert_eq!(result, Ok(3));
        assert_eq!(lines.len(), 4);
        assert_eq!(lines[0], CSV_HEADER);
        let columns = CSV_HEADER.split(',').count();
        for line in &lines[1..] {
            assert_eq!(line.split(',').count(), columns);
        }

        let first: Vec<&str> = lines[1].split(',').collect();
        assert_eq!(first[0], "0");
        assert_eq!(first[3], "");
        assert_eq!(first[5], "0.500000");
        assert_eq!(first[7], "");
        assert_ne!(first[10], "");

        let last: Vec<&str> = lines[3].split(',').collect();
        assert_eq!(last[0], "2");
        assert_ne!(last[3], "");
        assert_eq!(last[5], "");
        assert_ne!(last[7], "");

        let middle: Vec<&str> = lines[2].split(',').collect();
        assert!(middle.iter().all(|f| !f.is_empty()));
    }

    #[test]
    fn single_frame_has_only_current_columns() {
        let (result, lines) = run_on(&frame(10, 20));
        assert_eq!(result, Ok(1));
        let row: Vec<&str> = lines[1].split(',').collect();
        assert_eq!(row[3], "");
        assert_eq!(row[5], "");
        assert_eq!(row[7], "");
        assert_eq!(row[11], "");
        assert_eq!(row[10], "10.000000");
    }

    #[test]
    fn rejects_empty_input_and_truncated_last_frame() {
        let (result, lines) = run_on(&[]);
        assert!(result.is_err());
        assert!(lines.is_empty());

        let mut input = frame(1, 2);
        input.extend_from_slice(&frame(3, 4)[..10]);
        let (result, _) = run_on(&input);
        assert!(result.unwrap_err().contains("途中で終わっています"));
    }

    #[test]
    fn tracks_neighbours_across_the_sliding_window() {
        let input: Vec<u8> = [frame(10, 10), frame(20, 20), frame(40, 40), frame(80, 80)].concat();
        let (result, lines) = run_on(&input);
        assert_eq!(result, Ok(4));
        let sad_frame = |i: usize| lines[i + 1].split(',').nth(11).unwrap().to_string();
        assert_eq!(sad_frame(0), "");
        assert_eq!(sad_frame(1), "10.000000");
        assert_eq!(sad_frame(2), "20.000000");
        assert_eq!(sad_frame(3), "40.000000");
    }
}

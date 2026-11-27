mod args;
mod dump;

use std::io::{self, BufWriter, Write};
use std::process::ExitCode;

const USAGE: &str = "\
使い方:
  ivtc-cli dump --width <幅> --height <高さ> --field-order <tff|bff> [--coring <0-255>]

標準入力から NV12 の raw フレームを読み、フレームごとの特徴量を CSV で標準出力へ書き出す。

例:
  ffmpeg -ss <開始> -t <長さ> -i in.ts -an -fps_mode passthrough -pix_fmt nv12 -f rawvideo - \\
    | ivtc-cli dump --width 1440 --height 1080 --field-order tff > out.csv";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.split_first() {
        Some((command, rest)) if command == "dump" => run_dump(rest),
        _ => {
            eprintln!("{USAGE}");
            ExitCode::from(2)
        }
    }
}

fn run_dump(args: &[String]) -> ExitCode {
    let config = match args::parse(args) {
        Ok(config) => config,
        Err(message) => {
            eprintln!("ivtc-cli: {message}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };

    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut writer = BufWriter::new(stdout.lock());
    let result = dump::run(stdin.lock(), &mut writer, &config).and_then(|frames| {
        writer
            .flush()
            .map(|()| frames)
            .map_err(|e| format!("出力に失敗しました: {e}"))
    });

    match result {
        Ok(frames) => {
            eprintln!("ivtc-cli: {frames} フレームを処理しました");
            ExitCode::SUCCESS
        }
        Err(message) => {
            eprintln!("ivtc-cli: {message}");
            ExitCode::FAILURE
        }
    }
}

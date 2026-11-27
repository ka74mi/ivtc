# PlaybackIVTC

地上波アニメ(M2TS)を再生時に逆テレシネする DirectShow DMO(Rust / MSVC / windows-rs)。
現在は **ステージ1: NV12→NV12 のパススルー**(IVTC本体は未移植)。

## 構成
- `crates/ivtc-core` : DirectShow非依存のコア(NV12、format block解析、遅延キュー)。`cargo test` はどのOSでも通る
- `crates/ivtc-dmo` : DMO本体(cdylib)。COM/IMediaObject/登録だけを持つ薄い層
- `crates/ivtc-cli` : 解析用CLI(`dump`: フレームごとの特徴量を CSV で出力する。詳細は `docs/ivtc-cli-dump.md`)
- `crates/xtask`    : `cargo xtask ...`

## 使い方(管理者権限のPowerShellで)
    cargo xtask reinstall      # 解除 → ビルド → 登録
    cargo xtask check          # fmt / clippy / test

## トレース(フリーズ・接続不良の調査)
管理者権限のシェルで一度だけ設定し、対象アプリを再起動する(MPC-HCなど環境変数が渡らないアプリでも有効)。

    cargo xtask trace-on                  # %PUBLIC%\PlaybackIVTC\trace.log に出力(フォルダは自動作成)
    cargo xtask trace-on debug            # OutputDebugString(DebugView等)
    cargo xtask trace-off

環境変数 `PLAYBACK_IVTC_TRACE`(同じ書式)が設定されていればそちらを優先する。
`PLAYBACK_IVTC_DELAY=N` を設定すると N フレーム遅延のパススルーになる(EOS/Flush検証用)。

各行は `[exe名:PID:スレッド]` 付き。`> 名前` は入口、`< 名前 -> HRESULT` は出口。
入口だけで止まっていればその呼び出しの中でハングしている。

## テストの注意
- GraphStudioNext の出力側レンダラーは **Enhanced Video Renderer(またはVMR-9)** を使う。
  旧「Video Renderer」は最初の接続でRGBを要求するため、NV12のDMOとは素直に繋がらない。

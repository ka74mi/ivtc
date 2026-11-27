# WORKLOG

最終更新: 2026-11-27、記法: [確定] はログ・ソース・テストで確認済み、[報告] はユーザー報告、[仮説] は未検証の推測、[未確認] は未調査事項

## 1. 目的と方針
- 既存の VapourSynth C++ 版 IVTC (日本の地上波アニメ向け、24p 間引き + Weave) を、VapourSynth 非依存の Rust 製 DirectShow DMO として作り直す。再生プレイヤーは MPC-HC とする。
- 開発方針: 先にパススルー DMO を完成させて実機で確認する。その後、解析・ケイデンス検出・Weave を `ivtc-core` へ段階的に移植する。縞検出ロジックは差し替え可能な構造にする。
- 対象フォーマット: M2TS (地上波) の NV12・8 bit のみ。10 bit 対応は不要 (本人見込み)。

## 2. 環境
- 実行環境: Windows、sudo が利用できる PowerShell。Rust は stable 1.100.0 (x86_64-pc-windows-msvc)。
- 開発ディレクトリ: `C:\Users\ka74mi\workspace\ivtc`
- ビルド・テスト制約: Linux上の作業サンドボックスで `cargo check`、`cargo clippy`、`cargo test`、`cargo fmt` を実行できるのは `ivtc-core` と `xtask` のみ。`ivtc-dmo` は windows-core がLinuxでコンパイルできないため、ビルドもテストもWindows環境(実機またはWindowsのCI)でのみ確認できる。**Windows実機での動作確認はすべてユーザー環境に依存する。**
- 最新の修正版は `https://github.com/ka74mi/ivtc` を clone して取得する。

## 3. 構成 (ワークスペース)
- `crates/ivtc-core`: 純 Rust、DirectShow 非依存のコア ライブラリ。`nv12`、`video_info`、`delay_queue`、`passthrough` を実装済みで、テスト 22 件に合格。
- `crates/ivtc-dmo` (cdylib): COM の手書き実装 (`IMediaObject`)、レジストリ登録 (`DMORegister`、`VIDEO_EFFECT` カテゴリ、NV12 入出力)、トレース機能を含む DMO 本体。CLSID は `{D4B4B989-6B2A-4D0C-8C47-5D3F5CA5A7A1}`。
- `crates/ivtc-cli`: 現在は空。ステージ 2 で VapourSynth 版との数値比較用に実装予定。
- `crates/xtask`: ビルド・登録・トレース制御用のタスク ランナー。サブコマンドは `build`、`register`、`unregister`、`reinstall`、`check`、`trace-on [path|debug]`、`trace-off`。
- クレートとエディション: `windows` および `windows-core` は `=0.62.2` に固定し、Rust edition 2024 を使用。
- 遅延パススルー設定: 環境変数 `PLAYBACK_IVTC_DELAY` (既定値 `0`) で遅延フレーム数を指定可能。
- トレース設定: 環境変数 `PLAYBACK_IVTC_TRACE` を優先し、未設定なら HKLM `SOFTWARE\PlaybackIVTC` の `Trace` を参照する。既定のログ出力先は `%PUBLIC%\PlaybackIVTC\trace.log`。関数入口を `> 名前`、出口を `< 名前 -> HRESULT` で記録する。ログは追記されるため、測定前に毎回削除する。

## 4. 現在進行中の作業
- 目的: IVTC ロジックの設計を、実動画の解析結果を踏まえて確定する。
- 対象: `docs/prior-art-survey.md` (先行研究の調査結果)、`docs/ivtc-design-options.md` (設計候補の比較と未決事項)。
- 進捗: [確定] 論文・特許の調査と、設計候補の比較をドキュメントにまとめた。
- 未解決事項:
  - 位相推定の方式 (窓付きの系列推定、因果的な状態機械、併用) が未決。
  - 出力タイムスタンプの方式 (入力時刻の保持、24000/1001 への付け直し) が未決で、EVR での表示は [未確認]。
  - [報告] OP / ED の入りでケイデンスが変わるソースは稀で、サンプルが少ない。[仮説] 通常のソースを任意の位相でつなぐ疑似データで補う案がある。

## 5. 次のステップ
1. 特徴量ダンプ用の `ivtc-cli` を実装する。完了の判断基準は、録画 TS からフレームごとの特徴量を CSV で出力でき、`docs/scene-change-cadence-instability.md` の P / I / D ラベルと突き合わせられること。
2. 実動画の解析結果から設計を確定し、`docs/ivtc-design-options.md` に反映する。完了の判断基準は、未決事項 (位相推定の方式、出力タイムスタンプ、間引きの上限) に結論が出ていること。
3. 確定した設計で `ivtc-core` に IVTC を実装する。

## 6. 参考資料

- `docs/vapoursynth-ivtc-reference.md`
  - VapourSynth 版の実装詳細・アルゴリズム・既知の整理ポイント。
- `docs/mpc-hc-dmo-notes.md`
  - MPC-HC・DMO Wrapper・DirectShow に関する技術的な知見。
- `docs/scene-change-cadence-instability.md`
  - シーンチェンジ前後のケイデンス不安定に関する観測結果。
- `docs/prior-art-survey.md`
  - 論文、特許、TIVTC の調査結果と、別系統レポートとの照合。
- `docs/ivtc-design-options.md`
  - 設計候補の比較、実動画で確かめたいこと、未決事項。

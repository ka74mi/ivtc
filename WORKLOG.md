# WORKLOG

最終更新: 2026-11-23 / 記法: [確定]=ログ・ソース・テストで確認済み、[報告]=ユーザー報告、[仮説]=未検証の推測、[未確認]=調べていない事項

## 1. 目的と方針
- 既存のVapourSynth C++版IVTC(日本の地上波アニメ向け、24p間引き+Weave)を、VapourSynth非依存のRust製DirectShow DMOとして作り直す。再生プレイヤーはMPC-HCとする。
- 開発方針: 先にパススルーDMOを完成させて実機で確認する。その後、解析・ケイデンス検出・Weaveを `ivtc-core` へ段階的に移植する。縞検出ロジックは差し替え可能な構造にする。
- 対象フォーマット: M2TS(地上波)のNV12・8bitのみ。10bit対応は不要(本人見込み)。

## 2. 環境
- 実行環境: Windows、sudoが利用できるPowerShell。Rustは stable 1.100.0 (x86_64-pc-windows-msvc)。
- 開発ディレクトリ: `C:\Users\ka74mi\workspace\ivtc`
- ビルド・テスト制約: Linux上の作業サンドボックスでは `cargo check`、`cargo clippy`、`cargo test` `cargo rustfmt` のみ実行可能。**Windows実機での動作確認はすべてユーザー環境に依存する。**
- 最新の修正版は `https://github.com/ka74mi/ivtc` を clone して取得する。

## 3. 構成(ワークスペース)
- `crates/ivtc-core`: 純Rust、DirectShow非依存のコアライブラリ。`nv12`、`video_info`、`delay_queue`、`passthrough` を実装済みで、テスト22件に合格。
- `crates/ivtc-dmo`(cdylib): COMの手書き実装(`IMediaObject`)、レジストリ登録(`DMORegister`、`VIDEO_EFFECT` カテゴリ、NV12入出力)、トレース機能を含むDMO本体。CLSIDは `{D4B4B989-6B2A-4D0C-8C47-5D3F5CA5A7A1}`。
- `crates/ivtc-cli`: 現在は空。ステージ2でVapourSynth版との数値比較用に実装予定。
- `crates/xtask`: ビルド・登録・トレース制御用のタスクランナー。サブコマンドは `build`、`register`、`unregister`、`reinstall`、`check`、`trace-on [path|debug]`、`trace-off`。
- クレートとエディション: `windows` および `windows-core` は `=0.62.2` に固定し、Rust edition 2024 を使用。
- 遅延パススルー設定: 環境変数 `PLAYBACK_IVTC_DELAY`(既定値 `0`)で遅延フレーム数を指定可能。
- トレース設定: 環境変数 `PLAYBACK_IVTC_TRACE` を優先し、未設定ならHKLM `SOFTWARE\PlaybackIVTC` の `Trace` を参照する。既定のログ出力先は `%PUBLIC%\PlaybackIVTC\trace.log`。関数入口を `> 名前`、出口を `< 名前 -> HRESULT` で記録する。ログは追記されるため、測定前に毎回削除する。

## 4. 現在の状況
- [確定] パススルーDMOがMPC-HC上で動作する。フィルター構成は LAV Splitter Source → LAV Video → PlaybackIVTC DMO → EVR で、MPEG2-TSの映像が出力される。
- [確定] 以前の不具合の原因と修正内容は以下の通り。
  - `SetOutputType` が呼ばれるたびにキューを全破棄していた → キューを破棄しないよう修正。
  - `GetOutputType(0)` が入力型(stride 1920)を返し、EVRが選ぶ stride 2048 と食い違っていた → 設定済みの現在型を返すよう修正。
- [確定] 修正後のトレース(MPEG2-TS、約1786フレーム分)では、全フレームで `ProcessInput → ProcessOutput → 0x00000000` となり、出力タイムスタンプも入力と一致した。`0x80040204` などのエラーは発生していない。
- [確定] シーク時は別スレッド(スレッド2)から `Flush` が2回呼ばれ、1回目の呼び出しでキュー内の1フレームを破棄する(正常動作)。その直後にストリーミングスレッド側の `ProcessOutput` が `0x00000001` を返すが、これは割り込みによるもので無害。その後、シーク先からのフレームが正常に流れる。

## 6. 次のステップ
1. GitHub Actions の CI を実装する。まずは `rustfmt`、`check`、`clippy`、`test` の4点で、もし公式 GitHub Actions を利用する場合は `actions-lockfile` を利用する。バージョン更新には Renovate を利用する。簡易的なrenovate.jsonテンプレートは作成済み。
2. `PLAYBACK_IVTC_DELAY` を有効にした遅延パススルーで、シーク時の挙動、EOS時の吐き出し、音ズレの有無を確認する(`Flush`、`Discontinuity`、ドレイン処理の検証が目的)。
3. 遅延パススルーの検証に合格した後、IVTCロジックの実装についてVapourSynth版の既知の問題点を洗い出し、改善できる箇所がないか複数の論文などを元に再考する。
4. 再度まとめ直したあと、実装に取り掛かる。

## 7. 移植元(VapourSynth版)の要点
- 出力フレーム `n` を生成するごとに、`n-2` から `n+lookahead`(5、10、15のいずれか)までのフレーム窓を要求する。
- フィールド単位の16×16ブロックごとに縞検出を行う(`combMode=1`: 隣接行のmin/maxからの外れ値判定、`combMode=0`: 符号反転PCC)。
- 前フレームとの輝度SADをもとにテンポラルマスクを作成する。画面の上・左・右および下端の全幅は除外領域とする。
- ケイデンス状態機械は `PPPII` パターンを扱う。P位置の縞スコアは `-3` 倍、I位置は `+1` 倍としてロック判定を行い、ミスマッチの蓄積でアンロックする。
- Weave処理: `currentPhase==3` かつ `maxBlock > blockIThreshold/4` のとき、`FutureP1` の片フィールドを上書きする。出力フレームレートは `24000/1001` とする。
- 既知の整理ポイント: `lockThreshold`、`kernelSize`、`EvenLocked` は未使用。`computeIntraFramePcc` は常時計算されている。Weaveの `copyframe` に二重処理がある。
- 既知のVapourSynth版の問題点は以下の通り。
  - プレイヤーを終了させた時、先読み関連の影響か必ずメモリ関連のエラーを吐く。
  - 最適な縞検出閾値の未解決。解像度によっても最適な値が変わってしまう。
  - 通常本編中のケイデンスはCMを除き一定だが、イレギュラーなソースでは、本編 → OP、本編 → EDのシーンチェンジでもケイデンスが変わるものがある。
  この場合、シーンチェンジ後1サイクルほど奇妙なケイデンス周期が見られ、間引きに失敗する。
  詳細は `docs/scene-change-cadence-instability.md` に記している。
  
## 8. MPC-HC・DMOの知見
- [確定] MPC-HCはDMOを特別扱いせず、外部フィルターとしてモニカまたはCLSID経由で生成する。DMOは `DMOCATEGORY_VIDEO_EFFECT` の列挙対象となり、DMO Wrapper経由でNV12のみを受け取るため、LAV Videoの出力側に接続される。
- [確定] DMO Wrapperが扱うピン構成は1入力・複数出力まで。`ProcessOutput` はデータ到着時(EOSを含む)のみ呼ばれる。`S_FALSE` は「出力なし」を意味するため、`.ok()` 等でエラーや成功扱いと混同して潰さないよう注意する。
- [確定] EVRは `QueryAccept` を使ってstrideを選択する(本件では `2048` が選ばれた)。
- 参照資料: DMO Wrapper Filter (learn.microsoft.com/en-us/windows/win32/directshow/dmo-wrapper-filter)、IMediaObject::ProcessOutput (msdn ms785950)。

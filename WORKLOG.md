# WORKLOG

最終更新: 2026-11-23、記法: [確定] はログ・ソース・テストで確認済み、[報告] はユーザー報告、[仮説] は未検証の推測、[未確認] は未調査事項

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

## 4. 現在の状況
- [確定] パススルー DMO が MPC-HC 上で動作する。フィルター構成は LAV Splitter Source → LAV Video → PlaybackIVTC DMO → EVR で、MPEG2-TS の映像が出力される。
- [確定] 以前の不具合の原因と修正内容は以下のとおり。
  - `SetOutputType` が呼ばれるたびにキューを全破棄していた → キューを破棄しないよう修正。
  - `GetOutputType(0)` が入力型 (`stride` 1920) を返し、EVR が選ぶ `stride` 2048 と食い違っていた → 設定済みの現在型を返すよう修正。
- [確定] 修正後のトレース (MPEG2-TS、約 1786 フレーム分) では、全フレームで `ProcessInput → ProcessOutput → 0x00000000` となり、出力タイムスタンプも入力と一致した。`0x80040204` などのエラーは発生していない。
- [確定] シーク時は別スレッド (スレッド 2) から `Flush` が 2 回呼ばれ、1 回目の呼び出しでキュー内の 1 フレームを破棄する (正常動作)。その直後にストリーミング スレッド側の `ProcessOutput` が `0x00000001` を返すが、これは割り込みによるもので無害。その後、シーク先からのフレームが正常に流れる。

## 5. 次のステップ
1. GitHub Actions の CI を実装する。まずは `rustfmt`、`check`、`clippy`、`test` の 4 点で、公式 GitHub Actions を利用する場合は、`gh actions-lock` (拡張は `github/gh-actions-lock`) で `.github/workflows/actions.lock` を生成し、CI では `gh actions-lock --verify` で検証する。バージョン更新には Renovate を利用する。簡易的な `renovate.json` テンプレートは作成済み。
2. `PLAYBACK_IVTC_DELAY` を有効にした遅延パススルーで、シーク時の挙動、EOS 時の吐き出し、音ズレの有無を確認する (`Flush`、`Discontinuity`、ドレイン処理の検証が目的)。
3. 遅延パススルーの検証に合格した後、IVTC ロジックの実装について VapourSynth 版の既知の問題点を洗い出し、改善できる箇所がないか複数の論文などを元に再考する。
4. 再度まとめ直した後、実装に取り掛かる。

## 6. 移植元 (VapourSynth 版) の要点
- 出力フレーム `n` を生成するごとに、`n-2` から `n+lookahead` (5、10、15 のいずれか) までのフレーム窓を要求する。
- フィールド単位の 16×16 ブロックごとに縞検出を行う (`combMode=1`: 隣接行の `min/max` からの外れ値判定、`combMode=0`: 符号反転 PCC)。
- 前フレームとの輝度 SAD をもとにテンポラル マスクを作成する。画面の上・左・右および下端の全幅は除外領域とする。
- ケイデンス状態機械は `PPPII` パターンを扱う。P 位置の縞スコアは -3 倍、I 位置は +1 倍としてロック判定を行い、ミスマッチの蓄積でアンロックする。
- Weave 処理: `currentPhase==3` かつ `maxBlock > blockIThreshold/4` のとき、`FutureP1` の片フィールドを上書きする。出力フレーム レートは `24000/1001` とする。
- 既知の整理ポイント: `lockThreshold`、`kernelSize`、`EvenLocked` は未使用。`computeIntraFramePcc` は常時計算されている。Weave の `copyframe` に二重処理がある。
- 既知の VapourSynth 版の問題点は以下のとおり。
  - プレイヤーを終了したとき、先読み関連の影響か必ずメモリ関連のエラーを吐く。
  - 最適な縞検出閾値は未解決。解像度によっても最適な値が変わってしまう。
  - 通常本編中のケイデンスは CM を除き一定だが、イレギュラーなソースでは、本編 → OP、本編 → ED のシーン チェンジでもケイデンスが変わるものがある。
  この場合、シーン チェンジ後 1 サイクルほど奇妙なケイデンス周期が見られ、間引きに失敗する。
  詳細は `docs/scene-change-cadence-instability.md` に記している。

## 7. MPC-HC・DMO の知見
- [確定] MPC-HC は DMO を特別扱いせず、外部フィルターとしてモニカーまたは CLSID 経由で生成する。DMO は `DMOCATEGORY_VIDEO_EFFECT` の列挙対象となり、DMO Wrapper 経由で NV12 のみを受け取るため、LAV Video の出力側に接続される。
- [確定] DMO Wrapper が扱うピン構成は 1 入力・複数出力まで。`ProcessOutput` はデータ到着時 (EOS を含む) のみ呼ばれる。`S_FALSE` は「出力なし」を意味するため、`.ok()` 等でエラーや成功扱いと混同して潰さないよう注意する。
- [確定] EVR は `QueryAccept` を使って `stride` を選択する (本件では `2048` が選ばれた)。
- 参照資料: DMO Wrapper Filter (learn.microsoft.com/en-us/windows/win32/directshow/dmo-wrapper-filter)、IMediaObject::ProcessOutput (MSDN ms785950)。

## 8. 実装メモ(ソースコメント削除に伴う引き継ぎ)
[確定] `Passthrough` はフレームを入力時のレイアウトと一緒にキューへ積み、出力時に現在の出力レイアウトへstride変換コピーする。出力型の設定ではキューに触らない(DMO Wrapperは入力直後に、同じ型でも出力型を再設定してくるため)。入力の表示サイズが変わったときだけ、出力型を無効化してキューを捨てる。回帰テストは `resetting_output_layout_keeps_queued_frames`。
[確定] `DelayQueue` は、ストリーミング中に「入力を受け付ける」か「出力を出せる」のどちらかが常に成り立つようにする(DMOの規約)。EOS(Discontinuity)で `begin_drain` し、空になると通常状態に戻る。`Flush` は保持フレームを捨てる。
[確定] `IMediaObject` は手書き実装にしている。DMO Wrapper Filter はDMOを自分(外側)に集約して生成するため、非委譲 `IUnknown` と委譲 `IUnknown` の2つが必要で、windows-rs の `#[implement]` は集約に対応していない。vtableの関数は生の `HRESULT` を返す(`Result<()>` だと `S_FALSE` が潰れるため)。各呼び出しは `run()` を通り、入口/出口のトレース、Mutex、panic捕捉(FFIをまたぐpanicの防止)を行う。
[未確認] `ProcessOutput` は出力バッファの既存データの末尾に書き足す(DMOの規約。公式ドキュメントとの照合は未実施)。出力には常に `SYNCPOINT` を付ける(NV12は全フレームがキーフレーム)。
[未確認] `DMORegister` にNV12の入出力タイプを渡しているのは、`IFilterMapper2` やMPC-HCが「NV12を受けてNV12を出すフィルター」として見つけるため(公式ドキュメントとの照合は未実施)。登録は `HKCR` に書くため管理者権限が必要。
[確定] CLSID `{D4B4B989-6B2A-4D0C-8C47-5D3F5CA5A7A1}` は登録後に変更しない。
[確定] `PLAYBACK_IVTC_DELAY` の上限は32フレーム(`DELAY_MAX`)。
[確定] format block の解析では、YUVの場合 `biWidth` がストライド、`|biHeight|` がY平面の行数になる。表示領域は `rcSource` があればそれを使う(原点オフセットは無視)。UV平面はY平面(パディング込みの `plane_height` 行)の直後に続く。
[確定] `windows` と `windows-core` は `=0.62.2` に固定している。windows-rs はマイナー更新でAPIが動くことがあるため、上げるときは `cargo xtask check` が通ることを確認する。`windows-core` を直接依存にしているのは、`#[implement]` の展開結果が `windows_core::` を直接参照するため。
[確定] `.cargo/config.toml` で `crt-static` を指定している。他プロセス(MPC-HC、GraphStudioNext)にロードされるDLLなので、VC++ランタイムの再頒布パッケージに依存させないため。

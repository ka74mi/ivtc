## MPC-HC・DMO の知見
- [確定] MPC-HC は DMO を特別扱いせず、外部フィルターとしてモニカーまたは CLSID 経由で生成する。DMO は `DMOCATEGORY_VIDEO_EFFECT` の列挙対象となり、DMO Wrapper 経由で NV12 のみを受け取るため、LAV Video の出力側に接続される。
- [確定] DMO Wrapper が扱うピン構成は 1 入力・複数出力まで。`ProcessOutput` はデータ到着時 (EOS を含む) のみ呼ばれる。`S_FALSE` は「出力なし」を意味するため、`.ok()` 等でエラーや成功扱いと混同して潰さないよう注意する。
- [確定] EVR は `QueryAccept` を使って `stride` を選択する (本件では `2048` が選ばれた)。
- [確定] DMO Wrapper は入力直後に、同じ型でも `SetOutputType` を呼び直してくる。このためキューは出力型の設定で破棄してはならない。
- [確定] `IMediaObject` は手書きで実装している。DMO Wrapper は DMO を自分 (外側) に集約して生成するため、非委譲 `IUnknown` と委譲 `IUnknown` の2つが必要で、windows-rs の `#[implement]` は集約に対応していない。vtable の関数は生の `HRESULT` を返す (`Result<()>` だと `S_FALSE` が潰れる)。
- 参照資料: DMO Wrapper Filter (learn.microsoft.com/en-us/windows/win32/directshow/dmo-wrapper-filter)、IMediaObject::ProcessOutput (MSDN ms785950)。


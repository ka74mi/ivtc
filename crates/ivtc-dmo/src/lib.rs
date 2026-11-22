//! PlaybackIVTC DMO。
//!
//! - `exports`       : DLLのエクスポート(DllGetClassObject / DllCanUnloadNow)
//! - `registration`  : DllRegisterServer / DllUnregisterServer(COM + DMORegister)
//! - `class_factory` : IClassFactory
//! - `com_object`    : 集約(aggregation)対応のIMediaObject実装(COMの面倒はここだけ)
//! - `processor`     : COMに依存しない処理本体(型ネゴシエーション・キュー・コピー)
//! - `media_type`    : DMO_MEDIA_TYPE <-> ivtc-core::VideoInfo
//! - `trace`         : 環境変数 PLAYBACK_IVTC_TRACE によるトレース

#[macro_use]
mod trace;

mod class_factory;
mod class_id;
mod com_object;
mod exports;
mod media_type;
mod module_state;
mod processor;
mod registration;

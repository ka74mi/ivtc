use windows::core::GUID;

/// このCLSIDは固定。レジストリ登録後は変更しないこと。
pub(crate) const CLSID_PLAYBACK_IVTC_DMO: GUID =
    GUID::from_u128(0xD4B4B989_6B2A_4D0C_8C47_5D3F5CA5A7A1);

pub(crate) const FRIENDLY_NAME: &str = "PlaybackIVTC DMO";

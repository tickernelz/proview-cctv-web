pub mod client;
pub mod control;
pub mod crypto;
pub mod encode_osd;
#[cfg(test)]
pub(crate) mod fake;
pub mod frame;
pub mod playback;
pub mod settings;
pub mod storage_card;
pub mod talk;

pub const MAGIC: u8 = 0xff;
pub const HEADER_LEN: usize = 20;
pub const MAX_BODY: u32 = 8 * 1024 * 1024;

pub mod msg {
    pub const LOGIN: u16 = 1000;
    pub const LOGIN_REPLY: u16 = 1001;
    pub const LOGOUT: u16 = 1001;
    pub const KEEP_ALIVE: u16 = 1006;
    pub const SYSTEM_INFO: u16 = 1020;
    pub const CONFIG_SET: u16 = 1040;
    pub const CONFIG_GET: u16 = 1042;
    pub const CHANNEL_TITLE_SET: u16 = 1046;
    pub const CHANNEL_TITLE_GET: u16 = 1048;
    pub const TITLE_DOT_SET: u16 = 1050;
    pub const ABILITY: u16 = 1360;
    pub const PTZ_CONTROL: u16 = 1400;
    pub const TALK: u16 = 1430;
    pub const TALK_DATA: u16 = 1432;
    pub const TALK_CLAIM: u16 = 1434;
    pub const TIME_QUERY: u16 = 1452;
    pub const STORAGE_MANAGER: u16 = 1460;
    pub const SNAP: u16 = 1560;
    pub const MONITOR: u16 = 0x0585;
    pub const MONITOR_CHANNEL: u8 = 99;
    pub const MONITOR_START: u16 = 0x0582;
}

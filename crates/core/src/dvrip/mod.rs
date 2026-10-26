pub mod client;
pub mod crypto;
pub mod frame;

pub const MAGIC: u8 = 0xff;
pub const HEADER_LEN: usize = 20;
pub const MAX_BODY: u32 = 8 * 1024 * 1024;

pub mod msg {
    pub const LOGIN_REQUEST: u16 = 1000;
    pub const LOGOUT: u16 = 1001;
    pub const MONITOR_START: u16 = 1006;
    pub const MONITOR_STOP: u16 = 1007;
}

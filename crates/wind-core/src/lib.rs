//! Shared core for WindConnect's Windows and Android applications.

pub mod actions;
pub mod battery;
pub mod broadcast;
pub mod call;
pub mod clipboard;
pub mod discovery;
pub mod files;
pub mod find;
pub mod identity;
pub mod input;
pub mod media;
pub mod network;
pub mod notify;
pub mod pairing;
pub mod protocol;
pub mod relay;
pub mod session;
pub mod sms;
pub mod transport;
pub mod volume;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

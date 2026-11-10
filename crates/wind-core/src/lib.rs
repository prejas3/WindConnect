//! Shared core for WindConnect's Windows and Android applications.

pub mod broadcast;
pub mod discovery;
pub mod files;
pub mod identity;
pub mod input;
pub mod network;
pub mod pairing;
pub mod protocol;
pub mod relay;
pub mod session;
pub mod transport;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

//! Shared core for WindConnect's Windows and Android applications.

pub mod discovery;
pub mod identity;
pub mod input;
pub mod network;
pub mod protocol;
pub mod session;
pub mod transport;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

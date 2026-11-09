//! Shared core for WindConnect's Windows and Android applications.

pub mod identity;
pub mod protocol;
pub mod session;
pub mod transport;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

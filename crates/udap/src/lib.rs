//! Squeezebox UDAP (Universal Device Access Protocol) client.
//!
//! Ported from <https://github.com/yo61/go-udap> v2.4.8.

pub mod error;
pub mod mac;
pub mod protocol;
pub mod tlv;

pub use error::ProtocolError;
pub use mac::{Mac, MacParseError};
pub use protocol::{HEADER_SIZE, PORT, Packet};

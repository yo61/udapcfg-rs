//! Squeezebox UDAP (Universal Device Access Protocol) client.
//!
//! Ported from <https://github.com/yo61/go-udap> v2.4.8.

pub mod client;
pub mod device;
pub mod error;
pub mod getdata;
pub mod interfaces;
pub mod mac;
pub mod parameters;
pub mod protocol;
pub mod tlv;
pub mod transport;

pub use client::{Client, ClientError};
pub use device::Device;
pub use error::{EncodeError, GetDataError, ProtocolError};
pub use interfaces::{InterfaceError, NetInterface};
pub use mac::{Mac, MacParseError};
pub use protocol::{HEADER_SIZE, PORT, Packet};

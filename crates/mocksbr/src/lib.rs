//! In-process mock Squeezebox Receiver, for testing `udap` without hardware.

pub mod device;
pub mod network;
// Reply builders are an implementation detail of `Network`, which is
// the only caller; nothing outside the crate uses them. Narrowed rather
// than making `DeviceState` public purely to satisfy a signature.
pub(crate) mod responses;
mod state;
pub mod transport;
mod wire;

pub use device::DeviceConfig;
pub use network::Network;
pub use transport::MockTransport;

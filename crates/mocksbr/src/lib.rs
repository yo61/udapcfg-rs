//! In-process mock Squeezebox Receiver, for testing `udap` without hardware.

pub mod device;
pub mod network;
pub mod responses;
pub mod transport;

pub use device::DeviceConfig;
pub use network::Network;
pub use transport::MockTransport;

//! In-process mock Squeezebox Receiver, for testing `udap` without hardware.

pub mod device;
pub mod network;
// Reply builders are an implementation detail of `Network`, which is
// the only caller; nothing outside the crate uses them. Narrowed rather
// than making `DeviceState` public purely to satisfy a signature.
pub(crate) mod responses;
pub mod transport;
// Both are exercised by their own tests, but neither has a non-test
// caller until the state is wired into the network (M5-A Task 3) — so
// the expectation applies only to the non-test build, and fails once a
// caller appears.
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "callers arrive when the network routes writes, M5-A Task 3"
    )
)]
mod state;
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "callers arrive when the network routes writes, M5-A Task 3"
    )
)]
mod wire;

pub use device::DeviceConfig;
pub use network::Network;
pub use transport::MockTransport;

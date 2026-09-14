//! In-process mock Squeezebox Receiver, for testing `udap` without hardware.

pub mod device;
pub mod network;
pub mod responses;
pub mod transport;
// Its tests exercise every item, but no non-test caller exists until
// the state is wired into the network (M5-A Task 3) — so the
// expectation applies only to the non-test build, and fails once a
// caller appears.
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

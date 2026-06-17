use std::future::Future;
use std::time::Duration;

/// Time as a dependency so backoff is driven by a real clock in production and a
/// virtual one in tests (no real sleeping).
pub trait Clock {
    fn sleep(&self, duration: Duration) -> impl Future<Output = ()> + Send;
}

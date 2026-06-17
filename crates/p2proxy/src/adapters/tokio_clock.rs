use std::time::Duration;

use proxy_core::ports::Clock;

#[derive(Clone, Copy)]
pub struct TokioClock;

impl Clock for TokioClock {
    async fn sleep(&self, duration: Duration) {
        tokio::time::sleep(duration).await;
    }
}

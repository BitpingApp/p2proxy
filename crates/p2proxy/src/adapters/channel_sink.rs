use proxy_core::events::Events;
use proxy_core::ports::EventSink;
use tokio::sync::mpsc::Sender;

/// Forwards events to the TUI mpsc. `try_send` so a saturated or dropped
/// receiver (headless) is a cheap no-op rather than blocking the emitter.
#[derive(Clone)]
pub struct ChannelSink {
    tx: Sender<Events>,
}

impl ChannelSink {
    pub fn new(tx: Sender<Events>) -> Self {
        Self { tx }
    }
}

impl EventSink for ChannelSink {
    fn emit(&self, event: Events) {
        let _ = self.tx.try_send(event);
    }
}

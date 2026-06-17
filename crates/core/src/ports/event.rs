use crate::events::Events;

/// Best-effort UI/observability event sink. Production forwards to the TUI mpsc
/// (a no-op when headless); tests record into a buffer to assert on.
pub trait EventSink {
    fn emit(&self, event: Events);
}

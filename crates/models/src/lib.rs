use remoc::prelude::*;
use remoc::rtc::CallError;
use std::sync::Arc;
use tokio::sync::RwLock;

pub mod events;

// Custom error type that can convert from CallError.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub enum IncreaseError {
    Overflow,
    Call(CallError),
}

impl From<CallError> for IncreaseError {
    fn from(err: CallError) -> Self {
        Self::Call(err)
    }
}

// Trait defining remote service.
#[rtc::remote]
pub trait Counter {
    async fn value(&self) -> Result<u32, CallError>;

    async fn watch(&mut self) -> Result<rch::watch::Receiver<u32>, CallError>;

    #[no_cancel]
    async fn increase(&mut self, #[serde(default)] by: u32) -> Result<(), IncreaseError>;
}

// Server implementation object.
#[derive(Default)]
pub struct CounterObj {
    value: u32,
    watchers: Vec<rch::watch::Sender<u32>>,
}

impl CounterObj {
    pub fn new() -> Self {
        Self {
            value: 0,
            watchers: Vec::new(),
        }
    }
}

// Server implementation of trait methods.
#[rtc::async_trait]
impl Counter for CounterObj {
    async fn value(&self) -> Result<u32, CallError> {
        Ok(self.value)
    }

    async fn watch(&mut self) -> Result<rch::watch::Receiver<u32>, CallError> {
        let (tx, rx) = rch::watch::channel(self.value);
        self.watchers.push(tx);
        Ok(rx)
    }

    async fn increase(&mut self, by: u32) -> Result<(), IncreaseError> {
        match self.value.checked_add(by) {
            Some(new_value) => self.value = new_value,
            None => return Err(IncreaseError::Overflow),
        }

        for watch in &self.watchers {
            let _ = watch.send(self.value);
        }

        Ok(())
    }
}

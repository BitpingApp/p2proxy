use std::collections::HashMap;
use std::{collections::HashSet, sync::Arc};

use chrono::{DateTime, Local};
use config::ServerPeerOptions;
use dashmap::{DashMap, DashSet};
use libp2p::PeerId;
use remoc::robs::hash_map::{HashMapSubscription, ObservableHashMap};
use remoc::rtc::CallError;
use remoc::{prelude::*, rch::watch::Receiver};
use serde::{Deserialize, Serialize};

pub mod config;
pub mod events;
mod state;
pub use state::*;

use tokio::sync::RwLock;
use tracing::info;

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

pub type StateMap = ObservableHashMap<config::Server, ServerState>;
// Trait defining remote service.
#[rtc::remote]
pub trait Counter {
    async fn value(&self) -> Result<u64, CallError>;

    async fn add_state(&mut self) -> Result<(), CallError>;
    async fn watch(&mut self) -> Result<rch::watch::Receiver<u32>, CallError>;
    async fn subscribe(
        &self,
    ) -> Result<HashMapSubscription<config::Server, ServerState>, CallError>;

    // #[no_cancel]
    // async fn increase(&mut self, #[serde(default)] by: u32) -> Result<(), IncreaseError>;
}

#[derive(Debug, Serialize, Deserialize, Hash, Clone, PartialEq, PartialOrd, Ord, Eq, Default)]
pub struct BandwidthSlice {
    value: usize,
    at: DateTime<Local>,
}

#[derive(Debug, Serialize, Deserialize, Hash, Clone, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum ConnectionStatus {
    #[default]
    Disconnected,
    Connected,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct ServerState {
    // last_bw: BandwidthSlice,

    // status: ConnectionStatus,
    client_connections: usize,
    peer_connections: usize,
}

// Server implementation object.
#[derive(Debug)]
pub struct ServerContainer {
    value: StateMap,
    observers: Vec<rch::watch::Sender<u32>>,
}

impl ServerContainer {
    pub fn new(sessions: Vec<config::Server>) -> Self {
        let mut obs_map = ObservableHashMap::new();

        for x in sessions {
            obs_map.insert(x, Default::default());
        }

        Self {
            value: obs_map,
            observers: vec![],
        }
    }
}

// Server implementation of trait methods.
#[rtc::async_trait]
impl Counter for ServerContainer {
    async fn value(&self) -> Result<u64, CallError> {
        info!("Getting value");
        Ok(42)
    }

    async fn add_state(&mut self) -> Result<(), CallError> {
        self.value.insert(
            config::Server {
                protocol: config::ProxyProtocols::Socks5,
                port: 1000,
                peer_options: ServerPeerOptions {
                    destination_peer: None,
                    country: None,
                },
            },
            ServerState {
                // last_bw: BandwidthSlice {
                //     value: 500,
                //     at: Local::now(),
                // },
                // status: ConnectionStatus::Connected,
                client_connections: 0,
                peer_connections: 0,
            },
        );

        for observer in &self.observers {
            let _ = observer.send(self.observers.len() as u32);
        }

        Ok(())
    }

    async fn watch(&mut self) -> Result<rch::watch::Receiver<u32>, CallError> {
        info!("Get watcher");
        let (tx, rx) = rch::watch::channel(0);

        info!("Created tx, rx");
        self.observers.push(tx);

        info!("Sending rx");
        Ok(rx)
    }

    async fn subscribe(
        &self,
    ) -> Result<HashMapSubscription<config::Server, ServerState>, CallError> {
        Ok(self.value.subscribe(10))
    }

    // async fn increase(&mut self, by: u32) -> Result<(), IncreaseError> {
    //     match self.value.checked_add(by) {
    //         Some(new_value) => self.value = new_value,
    //         None => return Err(IncreaseError::Overflow),
    //     }

    //     for watch in &self.watchers {
    //         let _ = watch.send(self.value);
    //     }

    //     Ok(())
    // }
}

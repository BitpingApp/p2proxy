use std::collections::HashMap;

use chrono::{DateTime, Local};
use libp2p::PeerId;
use remoc::rtc::CallError;
use remoc::{prelude::*, rch};
use serde::{Deserialize, Serialize};

pub mod config;
pub mod events;

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
// Trait defining remote service.
#[rtc::remote]
pub trait Counter {
    async fn get_server_states(&self) -> Result<Vec<ServerStateInfo>, CallError>;
    async fn get_connection_status(&self) -> Result<String, CallError>;
    async fn get_stats(&self) -> Result<ProxyStats, CallError>;
    async fn watch_events(&mut self) -> Result<rch::mpsc::Receiver<events::Events>, CallError>;
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ServerStateInfo {
    pub server_id: String,
    pub protocol: String,
    pub port: u16,
    pub state: ServerState,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ProxyStats {
    pub total_sessions: usize,
    pub total_peers: usize,
    pub total_upload: u64,
    pub total_download: u64,
    pub connection_status: String,
    pub local_peer_id: Option<PeerId>,
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
    pub connection_status: ConnectionStatus,
    pub client_connections: usize,
    pub peer_connections: usize,
    pub total_upload: u64,
    pub total_download: u64,
    pub local_peer_id: Option<PeerId>,
}

// Server implementation object.
#[derive(Debug)]
pub struct ServerContainer {
    servers: HashMap<config::Server, ServerState>,
    event_senders: Vec<rch::mpsc::Sender<events::Events>>,
}

impl ServerContainer {
    pub fn new(sessions: Vec<config::Server>) -> Self {
        let mut servers = HashMap::new();

        for server in sessions {
            servers.insert(server, Default::default());
        }

        Self {
            servers,
            event_senders: vec![],
        }
    }

    pub async fn handle_event(&mut self, event: events::Events) {
        use events::*;
        
        // Send event to all subscribers first
        let mut failed_senders = Vec::new();
        for (idx, sender) in self.event_senders.iter().enumerate() {
            if sender.send(event.clone()).await.is_err() {
                failed_senders.push(idx);
            }
        }
        
        // Remove failed senders (clients disconnected)
        for idx in failed_senders.into_iter().rev() {
            self.event_senders.remove(idx);
        }
        
        // Update internal state
        match event {
            Events::LocalPeerId(peer_id) => {
                // Update local peer ID for all servers
                for state in self.servers.values_mut() {
                    state.local_peer_id = Some(peer_id);
                }
            }
            Events::Connection(connection_event) => {
                match connection_event {
                    ConnectionEvents::Connecting => {
                        for state in self.servers.values_mut() {
                            state.connection_status = ConnectionStatus::Disconnected;
                        }
                    }
                    ConnectionEvents::Connected(_peer_id) => {
                        for state in self.servers.values_mut() {
                            state.connection_status = ConnectionStatus::Connected;
                            state.peer_connections += 1;
                        }
                    }
                    ConnectionEvents::Disconnected => {
                        for state in self.servers.values_mut() {
                            state.connection_status = ConnectionStatus::Disconnected;
                            state.peer_connections = 0;
                        }
                    }
                }
            }
            Events::Session(session_event) => {
                match session_event {
                    SessionEvents::New(_session_id, _target_addr, _peer_id) => {
                        // Increment client connections for the first server (or all servers)
                        if let Some(state) = self.servers.values_mut().next() {
                            state.client_connections += 1;
                        }
                    }
                    SessionEvents::End(_session_id) => {
                        // Decrement client connections for the first server (or all servers)
                        if let Some(state) = self.servers.values_mut().next() {
                            if state.client_connections > 0 {
                                state.client_connections -= 1;
                            }
                        }
                    }
                }
            }
            Events::Bandwidth(bandwidth_event) => {
                match bandwidth_event {
                    BandwidthEvents::Upload(_session_id, bytes) => {
                        if let Some(state) = self.servers.values_mut().next() {
                            state.total_upload += bytes;
                        }
                    }
                    BandwidthEvents::Download(_session_id, bytes) => {
                        if let Some(state) = self.servers.values_mut().next() {
                            state.total_download += bytes;
                        }
                    }
                }
            }
        }
    }
}

// Server implementation of trait methods.
#[rtc::async_trait]
impl Counter for ServerContainer {
    async fn get_server_states(&self) -> Result<Vec<ServerStateInfo>, CallError> {
        info!("Getting server states");
        let mut result = Vec::new();
        for (server, state) in &self.servers {
            result.push(ServerStateInfo {
                server_id: format!("{}:{}", match &server.protocol {
                    config::ProxyProtocols::Socks5 => "socks5",
                }, server.port),
                protocol: format!("{:?}", &server.protocol),
                port: server.port,
                state: state.clone(),
            });
        }
        Ok(result)
    }

    async fn get_connection_status(&self) -> Result<String, CallError> {
        info!("Getting connection status");
        // Return the status of the first server or a general status
        if let Some(state) = self.servers.values().next() {
            match state.connection_status {
                ConnectionStatus::Connected => Ok("Connected".to_string()),
                ConnectionStatus::Disconnected => Ok("Disconnected".to_string()),
            }
        } else {
            Ok("No servers configured".to_string())
        }
    }

    async fn get_stats(&self) -> Result<ProxyStats, CallError> {
        info!("Getting proxy stats");
        let total_sessions = self.servers.values().map(|s| s.client_connections).sum();
        let total_peers = self.servers.values().map(|s| s.peer_connections).sum();
        let total_upload = self.servers.values().map(|s| s.total_upload).sum();
        let total_download = self.servers.values().map(|s| s.total_download).sum();
        
        let connection_status = if let Some(state) = self.servers.values().next() {
            match state.connection_status {
                ConnectionStatus::Connected => "Connected".to_string(),
                ConnectionStatus::Disconnected => "Disconnected".to_string(),
            }
        } else {
            "No servers configured".to_string()
        };
        
        let local_peer_id = self.servers.values().next().and_then(|s| s.local_peer_id);

        Ok(ProxyStats {
            total_sessions,
            total_peers,
            total_upload,
            total_download,
            connection_status,
            local_peer_id,
        })
    }

    async fn watch_events(&mut self) -> Result<rch::mpsc::Receiver<events::Events>, CallError> {
        info!("Creating event watcher");
        let (tx, rx) = rch::mpsc::channel(100);
        self.event_senders.push(tx);
        Ok(rx)
    }
}

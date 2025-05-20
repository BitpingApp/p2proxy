use std::{
    borrow::Cow,
    fmt::{write, Display},
    sync::Arc,
};

use color_eyre::eyre;
use figment::{
    providers::{Env, Format, Yaml},
    Figment,
};
use libp2p::{Multiaddr, PeerId};
use serde::{Deserialize, Serialize};

#[derive(Deserialize, Debug)]
pub struct Config {
    pub servers: Vec<Server>,
    pub port: u16,
    pub bitping_api_key: Cow<'static, str>,
}

#[derive(Serialize, Deserialize, Debug, Hash, Eq, PartialEq, PartialOrd, Ord, Clone)]
pub enum ProxyProtocols {
    Socks5,
}

#[derive(Serialize, Deserialize, Debug, Hash, Eq, PartialEq, PartialOrd, Ord, Clone)]
pub struct Server {
    pub protocol: ProxyProtocols,
    pub port: u16,

    #[serde(flatten)]
    pub peer_options: ServerPeerOptions,
}

#[derive(Serialize, Deserialize, Debug, Hash, Eq, PartialEq, PartialOrd, Ord, Clone)]
pub struct ServerPeerOptions {
    // TODO: Eventually replace this with some more options.
    pub destination_peer: Option<Multiaddr>,
    pub country: Option<String>,
}

impl Display for ServerPeerOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let opt_string = if let Some(p) = &self.destination_peer {
            format!("Destination Peer: {p:#?}")
        } else if let Some(c) = &self.country {
            format!("Country: {c}")
        } else {
            format!("Unknown")
        };

        write!(f, "{}", opt_string)
    }
}

impl Config {
    pub fn new() -> eyre::Result<Config> {
        Ok(Figment::new()
            .merge(Yaml::file("Config.yaml"))
            .merge(Env::raw())
            .extract()?)
    }
}

use std::{borrow::Cow, sync::Arc};

use color_eyre::eyre;
use figment::{
    providers::{Env, Format, Yaml},
    Figment,
};
use libp2p::{Multiaddr, PeerId};
use serde::Deserialize;

#[derive(Deserialize, Debug)]
pub struct Config {
    pub servers: Vec<Server>,
    pub port: u16,
    pub bitping_api_key: Cow<'static, str>,
    #[serde(default)]
    pub disable_ui: bool,
}

#[derive(Deserialize, Clone, Debug)]
pub enum ProxyProtocols {
    Socks5,
}

#[derive(Deserialize, Debug)]
pub struct Server {
    pub protocol: ProxyProtocols,
    pub port: u16,

    #[serde(flatten)]
    pub peer_options: ServerPeerOptions,
}

#[derive(Deserialize, Debug)]
pub struct ServerPeerOptions {
    // TODO: Eventually replace this with some more options.
    pub destination_peer: Option<Multiaddr>,
    pub country: Option<String>,
}

impl Config {
    pub fn new() -> eyre::Result<Config> {
        Ok(Figment::new()
            .merge(Yaml::file("Config.yaml"))
            .merge(Env::raw())
            .extract()?)
    }
}

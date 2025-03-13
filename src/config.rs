use color_eyre::eyre;
use figment::{
    providers::{Env, Format, Yaml},
    Figment,
};
use libp2p::Multiaddr;
use serde::Deserialize;

#[derive(Deserialize)]
pub struct Config {
    pub servers: Vec<Server>,

    pub destination_address: Multiaddr,
}

#[derive(Deserialize)]
pub enum ProxyProtocols {
    Socks5,
}

#[derive(Deserialize)]
pub struct Server {
    pub protocol: ProxyProtocols,
    pub port: u16,
}

impl Config {
    pub fn new() -> eyre::Result<Config> {
        Ok(Figment::new()
            .merge(Yaml::file("Config.yaml"))
            .merge(Env::raw())
            .extract()?)
    }
}

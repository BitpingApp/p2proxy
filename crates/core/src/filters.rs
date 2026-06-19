use serde::{Deserialize, Serialize};

use crate::config::ServerPeerOptions;

/// Tri-state node-attribute policy shared by the proxy / mobile / hosting
/// dimensions. `Allow` (the default) imposes no constraint; `Require` keeps
/// only nodes that have the attribute; `Deny` rejects nodes that have it.
#[derive(
    Serialize, Deserialize, Debug, Default, Hash, Eq, PartialEq, PartialOrd, Ord, Clone, Copy,
)]
#[serde(rename_all = "lowercase")]
pub enum FilterPolicy {
    #[default]
    Allow,
    Deny,
    Require,
}

/// Hub `FindNodes` selection criteria, projected from a server's
/// `ServerPeerOptions`. A pure value type: the mapping onto the wire
/// `Requirements`/`Exclusions` lives in the network actor so `proxy_core` stays
/// free of `protocols`.
#[derive(Debug, Default, Clone, Eq, PartialEq)]
pub struct NodeFilters {
    pub country: Option<String>,
    pub city: Option<String>,
    pub isp: Option<String>,
    pub asn: Option<u32>,
    pub proxy: FilterPolicy,
    pub mobile: FilterPolicy,
    /// Hosting (datacenter) policy. Hosting is the complement of the hub's
    /// `residential` attribute, so the actor maps this onto an inverted
    /// residential policy.
    pub hosting: FilterPolicy,
    pub min_bandwidth_bps: u128,
}

impl ServerPeerOptions {
    /// Project this server's discovery filters into the value type the network
    /// actor turns into a hub `FindNodes` request.
    pub fn node_filters(&self) -> NodeFilters {
        NodeFilters {
            country: self.country.clone(),
            city: self.city.clone(),
            isp: self.isp.clone(),
            asn: self.asn,
            proxy: self.proxy,
            mobile: self.mobile,
            hosting: self.hosting,
            min_bandwidth_bps: self.min_bandwidth.as_bps(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::FilterPolicy;

    #[test]
    fn filter_policy_deserialises_from_lowercase() {
        let parse = |s: &str| serde_json::from_str::<FilterPolicy>(s).expect("parses");
        assert_eq!(parse(r#""allow""#), FilterPolicy::Allow);
        assert_eq!(parse(r#""deny""#), FilterPolicy::Deny);
        assert_eq!(parse(r#""require""#), FilterPolicy::Require);
    }

    #[test]
    fn filter_policy_defaults_to_allow() {
        assert_eq!(FilterPolicy::default(), FilterPolicy::Allow);
    }
}

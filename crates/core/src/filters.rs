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

impl FilterPolicy {
    /// Human label for a non-default policy on `attribute` (e.g. `"proxy"`), or
    /// `None` when the policy imposes no constraint (`Allow`).
    pub fn constraint_label(&self, attribute: &str) -> Option<String> {
        match self {
            FilterPolicy::Allow => None,
            FilterPolicy::Deny => Some(format!("no {attribute}")),
            FilterPolicy::Require => Some(format!("{attribute} only")),
        }
    }
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

    /// Human-readable labels for the node-selection constraints set on this
    /// server, in a stable order. Empty when nothing constrains node attributes
    /// (the server matches any peer). Bandwidth, pinned peers, and stickiness
    /// are summarised separately by the caller.
    pub fn filter_labels(&self) -> Vec<String> {
        let mut labels = Vec::new();
        if let Some(country) = &self.country {
            labels.push(format!("country: {country}"));
        }
        if let Some(city) = &self.city {
            labels.push(format!("city: {city}"));
        }
        if let Some(isp) = &self.isp {
            labels.push(format!("isp: {isp}"));
        }
        if let Some(asn) = self.asn {
            labels.push(format!("ASN {asn}"));
        }
        if let Some(label) = self.proxy.constraint_label("proxy") {
            labels.push(label);
        }
        if let Some(label) = self.mobile.constraint_label("mobile") {
            labels.push(label);
        }
        if let Some(label) = self.hosting.constraint_label("hosting") {
            labels.push(label);
        }
        labels
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::StickyReconnect;
    use human_bandwidth::re::bandwidth::Bandwidth;

    fn opts(country: Option<&str>, asn: Option<u32>, hosting: FilterPolicy) -> ServerPeerOptions {
        ServerPeerOptions {
            destination_peers: None,
            fallback_to_discovery: false,
            sticky: true,
            sticky_reconnect: StickyReconnect::default(),
            country: country.map(str::to_string),
            min_bandwidth: Bandwidth::from_mbps(50),
            city: None,
            isp: None,
            asn,
            proxy: FilterPolicy::Allow,
            mobile: FilterPolicy::Allow,
            hosting,
        }
    }

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

    #[test]
    fn constraint_label_only_for_non_default() {
        assert_eq!(FilterPolicy::Allow.constraint_label("proxy"), None);
        assert_eq!(
            FilterPolicy::Deny.constraint_label("proxy"),
            Some("no proxy".to_string())
        );
        assert_eq!(
            FilterPolicy::Require.constraint_label("mobile"),
            Some("mobile only".to_string())
        );
    }

    #[test]
    fn filter_labels_lists_only_active_constraints() {
        let labels = opts(Some("NL"), Some(1136), FilterPolicy::Deny).filter_labels();
        assert_eq!(labels, vec!["country: NL", "ASN 1136", "no hosting"]);
    }

    #[test]
    fn filter_labels_empty_when_unconstrained() {
        assert!(
            opts(None, None, FilterPolicy::Allow)
                .filter_labels()
                .is_empty()
        );
    }
}

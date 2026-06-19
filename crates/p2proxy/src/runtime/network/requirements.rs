use protocols::models::v1::{NetworkPolicy, ProxyRestriction, Requirements};
use proxy_core::filters::{FilterPolicy, NodeFilters};

/// Build the hub `FindNodes` requirements from a server's node filters. String
/// and ASN filters become single-element requirement lists; the
/// proxy/mobile/hosting policies map onto the proto's tri-state enums. Hosting
/// is the complement of the hub's `residential` attribute, so it maps onto an
/// inverted residential policy.
pub fn requirements_from_filters(filters: &NodeFilters) -> Requirements {
    let mut requirements = Requirements::default();
    if let Some(country) = &filters.country {
        requirements.countries = vec![country.clone()];
    }
    if let Some(city) = &filters.city {
        requirements.cities = vec![city.clone()];
    }
    if let Some(isp) = &filters.isp {
        requirements.isps = vec![isp.clone()];
    }
    if let Some(asn) = filters.asn {
        requirements.asns = vec![asn];
    }
    requirements.set_proxy(proxy_restriction(filters.proxy));
    requirements.set_mobile(network_policy(filters.mobile));
    requirements.set_residential(residential_policy(filters.hosting));
    requirements
}

fn proxy_restriction(policy: FilterPolicy) -> ProxyRestriction {
    match policy {
        FilterPolicy::Allow => ProxyRestriction::Allow,
        FilterPolicy::Deny => ProxyRestriction::Disallow,
        FilterPolicy::Require => ProxyRestriction::Only,
    }
}

fn network_policy(policy: FilterPolicy) -> NetworkPolicy {
    match policy {
        FilterPolicy::Allow => NetworkPolicy::Allowed,
        FilterPolicy::Deny => NetworkPolicy::Denied,
        FilterPolicy::Require => NetworkPolicy::Required,
    }
}

/// Hosting is the inverse of the hub's `residential` attribute: requiring
/// hosting nodes denies residential ones, and denying hosting requires
/// residential ones.
fn residential_policy(hosting: FilterPolicy) -> NetworkPolicy {
    match hosting {
        FilterPolicy::Allow => NetworkPolicy::Allowed,
        FilterPolicy::Require => NetworkPolicy::Denied,
        FilterPolicy::Deny => NetworkPolicy::Required,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_string_and_asn_filters() {
        let filters = NodeFilters {
            country: Some("NL".into()),
            city: Some("Tiel".into()),
            isp: Some("KPN".into()),
            asn: Some(1136),
            ..Default::default()
        };
        let reqs = requirements_from_filters(&filters);
        assert_eq!(reqs.countries, vec!["NL"]);
        assert_eq!(reqs.cities, vec!["Tiel"]);
        assert_eq!(reqs.isps, vec!["KPN"]);
        assert_eq!(reqs.asns, vec![1136]);
    }

    #[test]
    fn empty_filters_leave_no_constraints() {
        let reqs = requirements_from_filters(&NodeFilters::default());
        assert!(reqs.countries.is_empty());
        assert!(reqs.asns.is_empty());
        assert_eq!(reqs.proxy(), ProxyRestriction::Allow);
        assert_eq!(reqs.mobile(), NetworkPolicy::Allowed);
        assert_eq!(reqs.residential(), NetworkPolicy::Allowed);
    }

    #[test]
    fn proxy_policy_maps_to_proxy_restriction() {
        assert_eq!(proxy_restriction(FilterPolicy::Allow), ProxyRestriction::Allow);
        assert_eq!(proxy_restriction(FilterPolicy::Deny), ProxyRestriction::Disallow);
        assert_eq!(proxy_restriction(FilterPolicy::Require), ProxyRestriction::Only);
    }

    #[test]
    fn hosting_policy_inverts_residential() {
        assert_eq!(residential_policy(FilterPolicy::Require), NetworkPolicy::Denied);
        assert_eq!(residential_policy(FilterPolicy::Deny), NetworkPolicy::Required);
        assert_eq!(residential_policy(FilterPolicy::Allow), NetworkPolicy::Allowed);
    }

    #[test]
    fn require_hosting_denies_residential_in_requirements() {
        let filters = NodeFilters {
            hosting: FilterPolicy::Require,
            ..Default::default()
        };
        assert_eq!(
            requirements_from_filters(&filters).residential(),
            NetworkPolicy::Denied
        );
    }
}

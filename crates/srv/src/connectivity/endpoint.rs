// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A model API address as the status names it: the same address written two ways is one service,
//! and a service's id and label never carry a credential or a query.

/// An address reduced to what identifies it: scheme, host, port and path, with no user, password,
/// query, fragment or trailing slash. `None` for one that doesn't parse as an address with a host
/// (`user:secret@proxy.example.com` is read as a scheme with no host): it is never shown or kept,
/// since there is no telling what in it is a credential, and the caller treats it as no address.
pub(super) fn normalise(url: &str) -> Option<String> {
    let mut parsed = reqwest::Url::parse(url.trim()).ok().filter(|u| u.has_host())?;
    let _ = parsed.set_username("");
    let _ = parsed.set_password(None);
    parsed.set_query(None);
    parsed.set_fragment(None);
    Some(parsed.as_str().trim_end_matches('/').to_string())
}

/// What tells a [`normalise`]d address apart in an id and a label: `host`, `host:port` when the port is
/// not the scheme's default, the path when there is one (two routes of one gateway are two services),
/// and the scheme when it is not `https`.
pub(super) fn host_of(url: &str) -> String {
    let Some(parsed) = reqwest::Url::parse(url).ok() else { return "unknown address".to_string() };
    let Some(host) = parsed.host_str() else { return "unknown address".to_string() };
    let scheme = if parsed.scheme() == "https" { String::new() } else { format!("{}://", parsed.scheme()) };
    let port = parsed.port().map(|p| format!(":{p}")).unwrap_or_default();
    let path = parsed.path().trim_end_matches('/');
    format!("{scheme}{host}{port}{path}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn n(url: &str) -> Option<String> {
        normalise(url)
    }

    #[test]
    fn the_same_address_written_two_ways_is_one() {
        assert_eq!(n("https://api.anthropic.com/"), n(" https://API.anthropic.com "));
        assert_eq!(n("https://api.anthropic.com").as_deref(), Some("https://api.anthropic.com"));
    }

    #[test]
    fn a_credential_a_query_and_a_fragment_are_dropped() {
        assert_eq!(n("https://user:secret@proxy.example.com:8443/v1/?key=abc#top").as_deref(), Some("https://proxy.example.com:8443/v1"));
    }

    #[test]
    fn a_path_is_kept() {
        assert_eq!(n("https://gateway.example.com/anthropic/").as_deref(), Some("https://gateway.example.com/anthropic"));
    }

    #[test]
    fn an_address_without_a_host_is_none_so_nothing_in_it_is_ever_shown() {
        for raw in ["user:secret@proxy.example.com:8080", "gw.example.com/v1?key=abc", "  not a url/ ", "mailto:a@b.c", ""] {
            assert_eq!(n(raw), None, "{raw}");
        }
    }

    #[test]
    fn the_host_has_its_port_only_when_it_is_not_the_scheme_default() {
        assert_eq!(host_of("https://proxy.example.com"), "proxy.example.com");
        assert_eq!(host_of("https://proxy.example.com:443"), "proxy.example.com");
        assert_eq!(host_of("http://localhost:4000"), "http://localhost:4000", "a scheme other than https is part of the identity");
        assert_eq!(host_of("user:secret@proxy"), "unknown address", "never echoes what it can't read");
    }

    #[test]
    fn two_routes_of_one_gateway_are_two_identities() {
        let a = host_of("https://gateway.example/team-a");
        let b = host_of("https://gateway.example/team-b");
        assert_eq!((a.as_str(), b.as_str()), ("gateway.example/team-a", "gateway.example/team-b"));
        assert_ne!(host_of("http://gateway.example"), host_of("https://gateway.example"));
    }
}

// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A model API address as the status names it: the same address written two ways is one service,
//! and a service's id and label never carry a credential or a query.

/// An address reduced to what identifies it: scheme, host, port and path, with no user, password,
/// query, fragment or trailing slash. An address that doesn't parse is kept as written (trimmed),
/// so a mistyped one still has its own row, and its probe says why it fails.
pub(super) fn normalise(url: &str) -> String {
    let url = url.trim();
    match reqwest::Url::parse(url) {
        Ok(mut parsed) if parsed.has_host() => {
            let _ = parsed.set_username("");
            let _ = parsed.set_password(None);
            parsed.set_query(None);
            parsed.set_fragment(None);
            parsed.as_str().trim_end_matches('/').to_string()
        }
        _ => url.trim_end_matches('/').to_string(),
    }
}

/// `host` or `host:port` of a [`normalise`]d address, for the id and the label.
pub(super) fn host_of(url: &str) -> String {
    match reqwest::Url::parse(url) {
        Ok(parsed) => match (parsed.host_str(), parsed.port()) {
            (Some(host), Some(port)) => format!("{host}:{port}"),
            (Some(host), None) => host.to_string(),
            _ => url.to_string(),
        },
        Err(_) => url.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_same_address_written_two_ways_is_one() {
        assert_eq!(normalise("https://api.anthropic.com/"), normalise(" https://API.anthropic.com "));
        assert_eq!(normalise("https://api.anthropic.com"), "https://api.anthropic.com");
    }

    #[test]
    fn a_credential_a_query_and_a_fragment_are_dropped() {
        assert_eq!(normalise("https://user:secret@proxy.example.com:8443/v1/?key=abc#top"), "https://proxy.example.com:8443/v1");
    }

    #[test]
    fn a_path_is_kept() {
        assert_eq!(normalise("https://gateway.example.com/anthropic/"), "https://gateway.example.com/anthropic");
    }

    #[test]
    fn an_address_that_does_not_parse_is_kept_as_written() {
        assert_eq!(normalise("  not a url/ "), "not a url");
        assert_eq!(host_of("not a url"), "not a url");
    }

    #[test]
    fn the_host_has_its_port_only_when_it_is_not_the_scheme_default() {
        assert_eq!(host_of("https://proxy.example.com"), "proxy.example.com");
        assert_eq!(host_of("https://proxy.example.com:443"), "proxy.example.com");
        assert_eq!(host_of("http://localhost:4000"), "localhost:4000");
    }
}

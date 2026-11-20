// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The sites an agent's browser pane may go to, when the agent limited it
//! with `OpenBrowser({allowed_origins})`
//! (docs/specs/SPEC_BROWSER_PANE_ALLOWED_ORIGINS_2026_10_09.md §2).
//!
//! Shared by srv, which checks the list it is given and re-checks every
//! request, and the CEF host, which stops a navigation off the list before it
//! happens. A list is kept in its normal form (`parse`): `https://example.com`,
//! `https://*.example.com`, `http://localhost:3000`.

/// Most entries one pane's list may have.
pub const MAX_ENTRIES: usize = 32;

/// `entry` in its normal form, or why it isn't a valid entry:
/// - `example.com` → `https://example.com`
/// - `*.example.com` → `https://*.example.com` (the domain and its subdomains)
/// - `https://example.com:8443/any/path` → `https://example.com:8443`
/// - `http://localhost:3000` → itself: `http` only when written out.
pub fn parse(entry: &str) -> Result<String, String> {
    let entry = entry.trim();
    if entry.is_empty() {
        return Err("an empty entry".to_string());
    }
    let (wildcard, rest) = match entry.strip_prefix("*.") {
        Some(rest) => (true, rest),
        None => (false, entry),
    };
    let with_scheme = if rest.contains("://") { rest.to_string() } else { format!("https://{rest}") };
    let bad = || format!("{entry:?} isn't a site (write example.com, *.example.com or https://example.com:8443)");
    let url = url::Url::parse(&with_scheme).map_err(|_| bad())?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(format!("{entry:?} isn't an http or https site"));
    }
    let Some(host) = url.host() else { return Err(bad()) };
    let origin = url.origin().ascii_serialization();
    if !wildcard {
        return Ok(origin);
    }
    match host {
        url::Host::Domain(d) if d.contains('.') => {
            // The origin with `*.` in front of its host.
            Ok(origin.replacen("://", "://*.", 1))
        }
        _ => Err(format!("{entry:?}: `*.` goes in front of a domain name with a dot in it")),
    }
}

/// Parse every entry of `entries`, deduplicated, in order. Fails on the
/// first bad entry, or past `MAX_ENTRIES`.
pub fn parse_list(entries: &[String]) -> Result<Vec<String>, String> {
    let mut out: Vec<String> = Vec::new();
    for e in entries {
        let p = parse(e)?;
        if !out.contains(&p) {
            out.push(p);
        }
    }
    if out.len() > MAX_ENTRIES {
        return Err(format!("at most {MAX_ENTRIES} sites"));
    }
    Ok(out)
}

/// The origin a navigation to `url` goes to, as an entry would name it
/// exactly (a `blob:` URL's is the origin it belongs to). `None` for an
/// address with no such origin (`about:`, `data:`).
pub fn target_origin(url: &str) -> Option<String> {
    let u = url::Url::parse(url).ok()?;
    match u.origin() {
        url::Origin::Tuple(scheme, _, _) if scheme == "http" || scheme == "https" => {
            Some(u.origin().ascii_serialization())
        }
        _ => None,
    }
}

/// `url` as an origin and nothing else: the origin ([`target_origin`]) when `url` names no more than one (no path
/// beyond `/`, no query, fragment or user information), else `None`. What a saved site is stored as when it is only
/// an origin written another way (`HTTPS://Example.com:443/` becomes `https://example.com`); an address with a path
/// is a login's own page and is left as it was typed.
pub fn bare_origin(url: &str) -> Option<String> {
    let u = url::Url::parse(url.trim()).ok()?;
    let bare = u.path() == "/" && u.query().is_none() && u.fragment().is_none() && u.username().is_empty() && u.password().is_none();
    if bare { target_origin(u.as_str()) } else { None }
}

/// Whether `a` and `b` name the same origin: the same scheme, host (any case) and port (the scheme's default when
/// absent), whatever else either carries (a trailing slash, a path, a query, user information). Never true for
/// another host, port or scheme. An address with no web origin is the same as another only when written the same.
pub fn same_origin(a: &str, b: &str) -> bool {
    match (target_origin(a.trim()), target_origin(b.trim())) {
        (Some(x), Some(y)) => x == y,
        _ => a.trim() == b.trim(),
    }
}

/// May a pane limited to `list` (normal form) go to `url`? `about:` pages
/// always; any other address with no web origin, never.
pub fn allows(list: &[String], url: &str) -> bool {
    if url::Url::parse(url).is_ok_and(|u| u.scheme() == "about") {
        return true;
    }
    let Some(target) = target_origin(url) else { return false };
    list.iter().any(|entry| entry_allows(entry, &target))
}

fn entry_allows(entry: &str, target: &str) -> bool {
    if entry == target {
        return true;
    }
    // `scheme://*.domain[:port]` matches the domain and any subdomain, on
    // that scheme and port.
    let Some((scheme, rest)) = entry.split_once("://*.") else { return false };
    let Some(t_rest) = target.strip_prefix(scheme).and_then(|t| t.strip_prefix("://")) else {
        return false;
    };
    let (domain, port) = split_port(rest);
    let (t_host, t_port) = split_port(t_rest);
    port == t_port && (t_host == domain || t_host.ends_with(&format!(".{domain}")))
}

/// `host[:port]` → (host, port), the port empty when there is none. An IPv6
/// host (`[::1]`) without a port is left whole.
fn split_port(s: &str) -> (&str, &str) {
    if s.ends_with(']') {
        return (s, "");
    }
    match s.rsplit_once(':') {
        Some((h, p)) if !h.is_empty() && !p.is_empty() && p.bytes().all(|c| c.is_ascii_digit()) => (h, p),
        _ => (s, ""),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_saved_site_and_a_page_origin_match_as_origins_not_as_strings() {
        // With and without the trailing slash, in both directions.
        assert!(same_origin("http://host.example:8080", "http://host.example:8080/"));
        assert!(same_origin("http://host.example:8080/", "http://host.example:8080"));
        // Upper-case scheme and host.
        assert!(same_origin("HTTPS://Example.COM", "https://example.com/"));
        // The scheme's default port is the same as none, and an explicit other port is another origin.
        assert!(same_origin("https://example.com:443", "https://example.com"));
        assert!(same_origin("http://example.com:80/", "http://example.com"));
        assert!(!same_origin("https://example.com:8443", "https://example.com"));
        assert!(!same_origin("http://example.com:8080", "http://example.com:8081"));
        // Another scheme is another origin: http is not https.
        assert!(!same_origin("http://example.com", "https://example.com"));
        // A look-alike host is another host, however it is written.
        assert!(!same_origin("https://example.com.evil.test", "https://example.com"));
        assert!(!same_origin("https://exarnple.com", "https://example.com"));
        assert!(!same_origin("https://example.com@evil.test", "https://example.com"), "the host is what follows the @");
        assert!(!same_origin("https://sub.example.com", "https://example.com"));
        // A path, query or user information is ignored for the match, and widens it to no other host or port.
        assert!(same_origin("https://example.com/login?next=/a", "https://example.com"));
        assert!(same_origin("https://user@example.com/app", "https://example.com/"));
        assert!(!same_origin("https://example.com/app", "https://example.com:8443"));
        // What has no web origin is the same only as written.
        assert!(same_origin("example.com", " example.com "));
        assert!(!same_origin("example.com", "https://example.com"));
    }

    #[test]
    fn a_site_that_is_only_an_origin_is_stored_in_one_form_and_a_login_page_is_left_alone() {
        assert_eq!(bare_origin("HTTPS://Example.com:443/").as_deref(), Some("https://example.com"));
        assert_eq!(bare_origin("http://host.example:8080/").as_deref(), Some("http://host.example:8080"));
        assert_eq!(bare_origin(" https://example.com ").as_deref(), Some("https://example.com"));
        assert_eq!(bare_origin("https://example.com/login"), None, "a path is the login's own page");
        assert_eq!(bare_origin("https://example.com/?a=1"), None);
        assert_eq!(bare_origin("https://user@example.com"), None, "user information is not dropped without being asked");
        assert_eq!(bare_origin("example.com"), None);
        assert_eq!(bare_origin("about:blank"), None);
    }

    #[test]
    fn entries_parse_to_their_normal_form() {
        assert_eq!(parse("example.com").unwrap(), "https://example.com");
        assert_eq!(parse(" Example.COM ").unwrap(), "https://example.com");
        assert_eq!(parse("*.example.com").unwrap(), "https://*.example.com");
        assert_eq!(parse("https://example.com:8443/a?b#c").unwrap(), "https://example.com:8443");
        assert_eq!(parse("https://example.com:443").unwrap(), "https://example.com");
        assert_eq!(parse("http://localhost:3000").unwrap(), "http://localhost:3000");
        assert_eq!(parse("example.com/some/path").unwrap(), "https://example.com");
        assert_eq!(parse("bücher.example").unwrap(), "https://xn--bcher-kva.example");
        assert_eq!(parse("http://[::1]:8080").unwrap(), "http://[::1]:8080");
        assert_eq!(parse("*.example.com:8443").unwrap(), "https://*.example.com:8443");
    }

    #[test]
    fn bad_entries_are_refused() {
        for bad in ["", "  ", "ftp://example.com", "javascript:alert(1)", "*.localhost", "*.192.0.2.1", "https://", "*."] {
            assert!(parse(bad).is_err(), "{bad:?}");
        }
        let many: Vec<String> = (0..=MAX_ENTRIES).map(|i| format!("s{i}.example.com")).collect();
        assert!(parse_list(&many).is_err());
        assert_eq!(parse_list(&["a.example".into(), "https://a.example/".into()]).unwrap(), vec!["https://a.example"]);
    }

    #[test]
    fn a_list_allows_its_sites_and_nothing_else() {
        let list = parse_list(&["example.com".into(), "*.docs.example.org".into(), "http://localhost:3000".into()]).unwrap();
        let ok = |u: &str| allows(&list, u);
        assert!(ok("https://example.com/"));
        assert!(ok("https://example.com/deep/path?q=1"));
        assert!(!ok("https://www.example.com/"), "a bare domain is just that host");
        assert!(!ok("http://example.com/"), "http only when written out");
        assert!(!ok("https://example.com:8443/"));
        assert!(ok("https://docs.example.org/"));
        assert!(ok("https://a.b.docs.example.org/"));
        assert!(!ok("https://evil-docs.example.org/"));
        assert!(!ok("https://docs.example.org.evil.example/"));
        assert!(ok("http://localhost:3000/app"));
        assert!(!ok("http://localhost:3001/"));
        assert!(!ok("https://evil.example/"));
        // A blob belongs to its origin.
        assert!(ok("blob:https://example.com/0f0e"));
        assert!(!ok("blob:https://evil.example/0f0e"));
        // about: pages are always fine; other non-web addresses never.
        assert!(ok("about:blank"));
        assert!(!ok("data:text/html,hi"));
        assert!(!ok("file:///etc/passwd"));
        assert!(!ok("not a url"));
    }

    #[test]
    fn a_wildcard_keeps_its_port() {
        let list = parse_list(&["*.example.com:8443".into()]).unwrap();
        assert!(allows(&list, "https://a.example.com:8443/"));
        assert!(!allows(&list, "https://a.example.com/"));
    }

    #[test]
    fn target_origin_is_what_allow_adds() {
        assert_eq!(target_origin("https://Evil.Example:443/x?y").as_deref(), Some("https://evil.example"));
        assert_eq!(target_origin("blob:https://a.example/id").as_deref(), Some("https://a.example"));
        assert_eq!(target_origin("about:blank"), None);
        assert_eq!(target_origin("data:text/html,x"), None);
        // What Allow adds then allows the same address.
        let added = vec![target_origin("https://new.example/path").unwrap()];
        assert!(allows(&added, "https://new.example/other"));
    }
}

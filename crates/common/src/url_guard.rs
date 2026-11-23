// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The origin of an address, asked through one guard.
//!
//! A `blob:` address carries another address (`blob:https://example.com/<id>`), and its origin is the
//! carried address's. An address that carries another `blob:` address has no origin here: it is
//! refused, so the answer is "no origin", and every caller treats that as "not allowed". Every place
//! that decides something from an address's origin asks through [`checked_origin`] (or
//! [`origin_string`]), never through `Url::origin` directly.

use url::{Origin, Url};

/// Whether `url` is a `blob:` address whose carried address is itself a `blob:` address.
///
/// The carried address is read once, the way `Url::origin` reads it, so the two never disagree about
/// what is carried; nothing here follows a chain.
pub fn is_nested_blob(url: &Url) -> bool {
    url.scheme() == "blob" && Url::parse(url.path()).is_ok_and(|inner| inner.scheme() == "blob")
}

/// `url`'s origin, or `None` for an address this guard refuses (a nested `blob:` address).
pub fn checked_origin(url: &Url) -> Option<Origin> {
    if is_nested_blob(url) {
        return None;
    }
    Some(url.origin())
}

/// [`checked_origin`] as `scheme://host[:port]` (`null` for an opaque origin), or `None` for a refused address.
pub fn origin_string(url: &Url) -> Option<String> {
    checked_origin(url).map(|origin| origin.ascii_serialization())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(s: &str) -> Url {
        Url::parse(s).unwrap()
    }

    /// `blob:` repeated `depth` times in front of `rest`.
    fn nested(depth: usize, rest: &str) -> String {
        format!("{}{rest}", "blob:".repeat(depth))
    }

    #[test]
    fn plain_addresses_keep_their_origin() {
        assert_eq!(origin_string(&url("https://example.com/a?b#c")).as_deref(), Some("https://example.com"));
        assert_eq!(origin_string(&url("http://localhost:3000/x")).as_deref(), Some("http://localhost:3000"));
        assert_eq!(origin_string(&url("https://example.com:443/")).as_deref(), Some("https://example.com"));
        assert_eq!(origin_string(&url("about:blank")).as_deref(), Some("null"));
        assert_eq!(origin_string(&url("data:text/plain,hi")).as_deref(), Some("null"));
    }

    #[test]
    fn an_ordinary_blob_address_keeps_the_origin_it_carries() {
        assert_eq!(origin_string(&url("blob:https://example.com/3f2a")).as_deref(), Some("https://example.com"));
        assert_eq!(origin_string(&url("blob:http://localhost:8080/3f2a")).as_deref(), Some("http://localhost:8080"));
        // A blob that carries no origin keeps having none.
        assert_eq!(origin_string(&url("blob:null/3f2a")).as_deref(), Some("null"));
        assert_eq!(origin_string(&url("blob:not-an-address")).as_deref(), Some("null"));
        assert_eq!(origin_string(&url("blob:data:text/plain,hi")).as_deref(), Some("null"));
        assert!(!is_nested_blob(&url("blob:https://example.com/3f2a")));
    }

    #[test]
    fn a_blob_address_that_carries_a_blob_address_has_no_origin() {
        for text in [
            nested(2, "https://example.com/3f2a"),
            nested(3, "https://example.com/3f2a"),
            "BLOB:Blob:https://example.com/3f2a".to_string(),
            "blob:\tblob:https://example.com/3f2a".to_string(),
            "blob: blob:https://example.com/3f2a".to_string(),
            nested(2, ""),
        ] {
            let u = url(&text);
            assert!(is_nested_blob(&u), "{text:?}");
            assert_eq!(checked_origin(&u), None, "{text:?}");
            assert_eq!(origin_string(&u), None, "{text:?}");
        }
    }

    #[test]
    fn deeply_nested_input_is_refused_and_the_stack_holds() {
        // Large enough that following the chain would not fit in a thread's stack.
        for depth in [100, 10_000, 1_000_000] {
            let u = url(&nested(depth, "https://example.com/3f2a"));
            assert_eq!(checked_origin(&u), None, "depth {depth}");
        }
        // The same on a thread with a small stack: the guard reads one level and stops.
        let handle = std::thread::Builder::new()
            .stack_size(64 * 1024)
            .spawn(|| origin_string(&url(&nested(1_000_000, "https://example.com/3f2a"))))
            .unwrap();
        assert_eq!(handle.join().unwrap(), None);
    }

    #[test]
    fn a_blob_address_inside_another_scheme_is_not_a_nested_blob() {
        let u = url("https://example.com/blob:https://other.example/x");
        assert_eq!(origin_string(&u).as_deref(), Some("https://example.com"));
        assert!(!is_nested_blob(&u));
    }
}

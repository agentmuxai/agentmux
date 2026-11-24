//! How a page address is shown to an agent (in an action's answer, a wait, a snapshot header, the action log).
//!
//! A sign-in redirect carries one-time values in its query or fragment: the request's state and nonce, the
//! authorization code and its code challenge, or a token. An agent that reads the whole address has them in its
//! context, its transcript and anything it quotes. So an address whose query or fragment carries one of those keys
//! is shown as its scheme, host and path, with the query shown as `?...` and the fragment as `#...`. Any other
//! address keeps its query and fragment: a search (`?q=`) or a record (`?id=`) is what the agent needs to read.
//! A user name and password in the authority are never shown.

/// Keys (lowercase) that make an address one to cut.
const SECRET_KEYS: &[&str] = &[
    "state",
    "nonce",
    "code",
    "code_challenge",
    "code_verifier",
    "id_token",
    "access_token",
    "refresh_token",
    "session",
    "session_id",
    "sessionid",
    "token",
    "continue",
    "client_secret",
    "id_token_hint",
    "samlrequest",
    "samlresponse",
    "relaystate",
];

/// `url` as an agent may read it. Every place a tool prints a page address goes through this.
pub fn shown_url(url: &str) -> String {
    let (before_fragment, fragment) = match url.split_once('#') {
        Some((a, b)) => (a, Some(b)),
        None => (url, None),
    };
    let (base, query) = match before_fragment.split_once('?') {
        Some((a, b)) => (a, Some(b)),
        None => (before_fragment, None),
    };
    let base = without_userinfo(base);
    // A single-page app keeps its parameters after a `?` inside the fragment (`#/callback?code=...`).
    let fragment_params = fragment.map(|f| f.split_once('?').map_or(f, |(_, q)| q));
    let cut = query.is_some_and(carries_secret) || fragment_params.is_some_and(carries_secret);
    let mut out = base;
    if cut {
        if query.is_some() {
            out.push_str("?...");
        }
        if fragment.is_some() {
            out.push_str("#...");
        }
        return out;
    }
    if let Some(q) = query {
        out.push('?');
        out.push_str(q);
    }
    if let Some(f) = fragment {
        out.push('#');
        out.push_str(f);
    }
    out
}

/// `scheme://host/path` without a `user:password@` in the authority.
fn without_userinfo(base: &str) -> String {
    let Some((scheme, rest)) = base.split_once("://") else { return base.to_string() };
    let (authority, path) = match rest.find('/') {
        Some(i) => rest.split_at(i),
        None => (rest, ""),
    };
    let host = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    format!("{scheme}://{host}{path}")
}

/// Whether a `a=b&c=d` list has a pair whose key is one to cut. A bare word is not a pair (`#code` is an anchor).
fn carries_secret(params: &str) -> bool {
    params.split(['&', ';']).any(|pair| {
        let Some((key, _)) = pair.split_once('=') else { return false };
        let key = percent_decode(key).to_ascii_lowercase();
        SECRET_KEYS.contains(&key.as_str())
    })
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = |b: u8| (b as char).to_digit(16);
        match bytes[i] {
            b'%' if i + 2 < bytes.len() && hex(bytes[i + 1]).is_some() && hex(bytes[i + 2]).is_some() => {
                out.push((hex(bytes[i + 1]).unwrap_or(0) * 16 + hex(bytes[i + 2]).unwrap_or(0)) as u8);
                i += 3;
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::shown_url;

    #[test]
    fn a_provider_authorize_address_is_cut_to_its_path() {
        let u = "https://idp.example/oauth2/authorize?client_id=abc&redirect_uri=https%3A%2F%2Fapp.example%2Fcb\
                 &response_type=code&state=S3CRET&nonce=N0NCE&code_challenge=CHAL&code_challenge_method=S256";
        assert_eq!(shown_url(u), "https://idp.example/oauth2/authorize?...");
    }

    #[test]
    fn a_token_in_the_fragment_is_cut() {
        assert_eq!(shown_url("https://app.example/cb#access_token=TOK&token_type=bearer&state=S"), "https://app.example/cb#...");
        assert_eq!(shown_url("https://app.example/#/callback?code=ONE"), "https://app.example/#...");
    }

    #[test]
    fn a_search_and_a_record_keep_their_query() {
        assert_eq!(shown_url("https://shop.example/search?q=red+shoes&page=2"), "https://shop.example/search?q=red+shoes&page=2");
        assert_eq!(shown_url("https://shop.example/orders?id=1042#items"), "https://shop.example/orders?id=1042#items");
    }

    #[test]
    fn a_sign_in_address_with_no_query_is_unchanged() {
        assert_eq!(shown_url("https://idp.example/signin"), "https://idp.example/signin");
        assert_eq!(shown_url("https://docs.example/guide#code"), "https://docs.example/guide#code");
    }

    #[test]
    fn an_ordinary_host_carrying_a_code_is_cut() {
        assert_eq!(shown_url("https://shop.example/return?id=7&code=ABC123"), "https://shop.example/return?...");
        assert_eq!(shown_url("https://shop.example/next?continue=https%3A%2F%2Fshop.example%2Fa%3Fx%3D1"), "https://shop.example/next?...");
    }

    #[test]
    fn a_key_is_matched_whatever_its_case_or_encoding() {
        assert_eq!(shown_url("https://x.example/p?Code=1"), "https://x.example/p?...");
        assert_eq!(shown_url("https://x.example/p?st%61te=1"), "https://x.example/p?...");
        assert_eq!(shown_url("https://x.example/p?statement=1&codex=2"), "https://x.example/p?statement=1&codex=2");
    }

    #[test]
    fn a_user_name_and_password_are_never_shown() {
        assert_eq!(shown_url("https://me:hunter2@host.example/a?q=1"), "https://host.example/a?q=1");
        assert_eq!(shown_url("https://me:hunter2@host.example"), "https://host.example");
    }

    #[test]
    fn what_is_not_an_address_passes_through() {
        assert_eq!(shown_url(""), "");
        assert_eq!(shown_url("about:blank"), "about:blank");
    }
}

// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use axum::Router;
use tower::ServiceExt;

use crate::server::routes::with_frontend;
use crate::server::tests::test_state;

/// The folder a packaged server serves: every file carries the one fixed date the package was built with.
fn build(index: &str, boot: &str, asset_name: &str, asset: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("index.html"), index).unwrap();
    std::fs::write(dir.path().join("early-boot.js"), boot).unwrap();
    std::fs::create_dir(dir.path().join("assets")).unwrap();
    std::fs::write(dir.path().join("assets").join(asset_name), asset).unwrap();
    std::fs::create_dir(dir.path().join("fonts")).unwrap();
    std::fs::write(dir.path().join("fonts/f.woff2"), "font").unwrap();
    dir
}

fn router(dir: &std::path::Path) -> Router {
    with_frontend(Router::new(), dir).with_state(test_state())
}

struct Got {
    status: StatusCode,
    headers: axum::http::HeaderMap,
    body: String,
}

async fn get(router: &Router, uri: &str, headers: &[(&str, &str)]) -> Got {
    let mut req = Request::builder().uri(uri).header(header::HOST, "127.0.0.1:7777");
    for (k, v) in headers {
        req = req.header(*k, *v);
    }
    let resp = router.clone().oneshot(req.body(Body::empty()).unwrap()).await.unwrap();
    let status = resp.status();
    let headers = resp.headers().clone();
    let body = String::from_utf8_lossy(&axum::body::to_bytes(resp.into_body(), 1 << 20).await.unwrap()).into_owned();
    Got { status, headers, body }
}

fn h<'a>(got: &'a Got, name: &str) -> Option<&'a str> {
    got.headers.get(name).and_then(|v| v.to_str().ok())
}

#[tokio::test]
async fn the_page_and_the_files_without_a_hash_are_revalidated_and_carry_a_validator_and_no_date() {
    let dir = build("<html>shell A</html>", "boot A", "app-abc123.js", "app A");
    let r = router(dir.path());
    for uri in ["/", "/early-boot.js", "/fonts/f.woff2", "/some/view"] {
        let got = get(&r, uri, &[]).await;
        assert_eq!(got.status, StatusCode::OK, "{uri}");
        assert_eq!(h(&got, "cache-control"), Some("no-cache"), "{uri}");
        assert!(h(&got, "etag").is_some_and(|e| e.starts_with('"') && e.ends_with('"')), "{uri}: {:?}", got.headers);
        assert!(h(&got, "last-modified").is_none(), "{uri}: a fixed date is what pinned the old shell");
    }
}

#[tokio::test]
async fn a_file_under_assets_is_cached_for_good_and_a_missing_one_is_not_the_page_cached_for_good() {
    let dir = build("<html>shell A</html>", "boot A", "app-abc123.js", "app A");
    let r = router(dir.path());
    let got = get(&r, "/assets/app-abc123.js", &[]).await;
    assert_eq!(got.body, "app A");
    assert_eq!(h(&got, "cache-control"), Some("private, max-age=31536000, immutable"));
    // No such file: the single-page app's fallback answers with the page, which must not be kept for a year.
    let missing = get(&r, "/assets/gone-ffff.js", &[]).await;
    assert!(missing.body.contains("shell A"), "{}", missing.body);
    assert_eq!(h(&missing, "cache-control"), Some("no-cache"));
}

#[tokio::test]
async fn an_unchanged_file_is_a_304_by_its_validator_and_a_changed_one_is_sent() {
    let dir = build("<html>shell A</html>", "boot A", "app-abc123.js", "app A");
    let r = router(dir.path());
    let first = get(&r, "/", &[]).await;
    let etag = h(&first, "etag").unwrap().to_string();
    let again = get(&r, "/", &[("if-none-match", &etag)]).await;
    assert_eq!(again.status, StatusCode::NOT_MODIFIED);
    assert!(again.body.is_empty());
    assert_eq!(h(&again, "etag"), Some(etag.as_str()));
    let listed = get(&r, "/", &[("if-none-match", &format!("\"nope\", W/{etag}"))]).await;
    assert_eq!(listed.status, StatusCode::NOT_MODIFIED, "a list, and a weak form, name the tag");
    let other = get(&r, "/", &[("if-none-match", "\"something-else\"")]).await;
    assert_eq!(other.status, StatusCode::OK);
}

/// The case of the report: a browser loaded build A, the server was upgraded to build B on the same address, and
/// the browser's next load of the page must be B's shell, whether it asks by the tag of A or (an older server's file,
/// cached with only its date) by the fixed date.
#[tokio::test]
async fn after_an_upgrade_on_the_same_address_the_next_load_is_the_new_shell() {
    let a = build("<html>shell A, app-aaa.js</html>", "boot A", "app-aaa.js", "app A");
    let before = get(&router(a.path()), "/", &[]).await;
    let tag_a = h(&before, "etag").unwrap().to_string();
    assert!(before.body.contains("shell A"));

    let b = build("<html>shell B, app-bbb.js</html>", "boot A", "app-bbb.js", "app B");
    let r = router(b.path());
    let by_tag = get(&r, "/", &[("if-none-match", &tag_a)]).await;
    assert_eq!(by_tag.status, StatusCode::OK, "the tag of A names nothing of B");
    assert!(by_tag.body.contains("shell B"), "{}", by_tag.body);
    assert_ne!(h(&by_tag, "etag"), Some(tag_a.as_str()));

    let by_date = get(&r, "/", &[("if-modified-since", "Thu, 01 Jan 2026 00:00:00 GMT")]).await;
    assert_eq!(by_date.status, StatusCode::OK, "a browser holding an older server's page asks by the date only");
    assert!(by_date.body.contains("shell B"));

    // The page that did not change between the builds is still a 304.
    let boot_a = get(&router(a.path()), "/early-boot.js", &[]).await;
    let boot_b = get(&r, "/early-boot.js", &[("if-none-match", h(&boot_a, "etag").unwrap())]).await;
    assert_eq!(boot_b.status, StatusCode::NOT_MODIFIED);
}

#[tokio::test]
async fn a_head_request_gets_the_rule_but_no_validator_of_an_empty_body() {
    let dir = build("<html>shell A</html>", "boot A", "app-abc123.js", "app A");
    let r = router(dir.path());
    for uri in ["/", "/early-boot.js"] {
        let resp = r
            .clone()
            .oneshot(Request::builder().method("HEAD").uri(uri).header(header::HOST, "127.0.0.1:7777").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK, "{uri}");
        assert_eq!(resp.headers().get("cache-control").and_then(|v| v.to_str().ok()), Some("no-cache"), "{uri}");
        assert!(resp.headers().get("etag").is_none(), "{uri}: a tag of no bytes is the same for every file");
    }
}

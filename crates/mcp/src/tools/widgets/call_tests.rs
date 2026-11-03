// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What each widget tool sends to srv and what it tells the agent back, pinned against a
//! stand-in srv, so `call` can be split into one function per tool without changing either.

use super::super::test_stub::{stub_srv, Fixture};
use super::*;

/// A path that is absolute on whichever machine the test runs on.
fn absolute(name: &str) -> String {
    std::env::temp_dir().join(name).to_string_lossy().into_owned()
}

/// One approved widget with one view, in the shape srv lists it; also holds the `block_id` srv
/// answers a pane open with, since the stand-in answers every request the same way.
fn widgets(state: &str) -> Value {
    json!({
        "packages": [{
            "id": "a.b", "name": "B", "version": "1.0.0", "kind": "sandboxed", "state": state, "permissions": ["net:x"],
            "error": null, "folder": "/f", "panes": [{ "view": "ext:a.b/main", "default_meta": { "d": 1 } }]
        }],
        "block_id": "b9"
    })
}

#[tokio::test]
async fn the_list_gives_each_widget_with_its_views() {
    let srv = stub_srv("200 OK", widgets("approved"));
    let fx = Fixture::new(&srv);
    let said: Value = serde_json::from_str(&call("WidgetList", &json!({}), &fx.cx()).await.unwrap()).unwrap();
    assert_eq!(
        said,
        json!({ "widgets": [{
            "id": "a.b", "name": "B", "version": "1.0.0", "kind": "sandboxed", "state": "approved", "permissions": ["net:x"],
            "error": null, "folder": "/f", "views": ["ext:a.b/main"]
        }] })
    );
    assert_eq!(srv.seen.lock().unwrap().iter().map(|(p, _)| p.as_str()).collect::<Vec<_>>(), ["/api/v1/widgets"]);
    drop(fx);
    // No packages is an empty list.
    let srv = stub_srv("200 OK", json!({}));
    let fx = Fixture::new(&srv);
    let said: Value = serde_json::from_str(&call("WidgetList", &json!({}), &fx.cx()).await.unwrap()).unwrap();
    assert_eq!(said, json!({ "widgets": [] }));
}

#[tokio::test]
async fn an_install_sends_the_path_the_cap_on_the_wait_and_who_asks() {
    let srv = stub_srv("200 OK", json!({ "status": "installed", "id": "a.b" }));
    let fx = Fixture::new(&srv);
    // An absolute path on this machine goes as it is.
    let path = absolute("w");
    let said = call("WidgetInstall", &json!({ "path": path }), &fx.cx()).await.unwrap();
    assert!(said.contains("\"status\": \"installed\"") && !said.contains("\"message\""), "{said}");
    call("WidgetInstall", &json!({ "path": path, "replace": false, "wait_secs": 9999 }), &fx.cx()).await.unwrap();
    call("WidgetInstall", &json!({ "path": path, "wait_secs": 5 }), &fx.cx()).await.unwrap();
    let seen = srv.seen.lock().unwrap().clone();
    assert!(seen.iter().all(|(p, _)| p == "/api/v1/widgets/install"));
    let bodies: Vec<&Value> = seen.iter().map(|(_, b)| b).collect();
    assert_eq!((bodies[0]["path"].as_str(), bodies[0]["replace"].clone(), bodies[0]["wait_secs"].clone()), (Some(path.as_str()), json!(true), json!(300)));
    assert_eq!((bodies[1]["replace"].clone(), bodies[1]["wait_secs"].clone()), (json!(false), json!(600)));
    assert_eq!(bodies[2]["wait_secs"], 5);
    // A container agent is attributed by its token: the proof it sends names it and carries no signature.
    assert_eq!((bodies[0]["auth"]["agent_id"].as_str(), bodies[0]["auth"]["sig"].as_str()), (Some("tester"), Some("")));
}

#[tokio::test]
async fn a_relative_path_is_taken_from_the_agents_workspace() {
    let srv = stub_srv("200 OK", json!({ "status": "installed" }));
    let fx = Fixture::new(&srv);
    let workdir = absolute("space");
    std::env::set_var("AGENTMUX_AGENT_WORKDIR", &workdir);
    let result = call("WidgetInstall", &json!({ "path": "my-widget" }), &fx.cx()).await;
    std::env::remove_var("AGENTMUX_AGENT_WORKDIR");
    result.unwrap();
    let want = std::path::Path::new(&workdir).join("my-widget").to_string_lossy().into_owned();
    assert_eq!(srv.seen.lock().unwrap()[0].1["path"], want.as_str());
}

#[tokio::test]
async fn an_install_that_needs_the_super_agent_says_what_happens_next() {
    let srv = stub_srv("200 OK", json!({ "status": "requested", "review": "r-7" }));
    let fx = Fixture::new(&srv);
    let said: Value = serde_json::from_str(&call("WidgetInstall", &json!({ "path": absolute("w") }), &fx.cx()).await.unwrap()).unwrap();
    assert_eq!(said["status"], "requested");
    let message = said["message"].as_str().unwrap();
    assert!(message.starts_with("Asked for the install as a sensitive operation (r-7). If the super agent approves, the person sees a countdown"), "{message}");
    assert!(message.ends_with("Widget list shows when it is approved; then open it."), "{message}");
    assert!(!message.contains("  "), "no runs of spaces: {message:?}");
}

#[tokio::test]
async fn an_install_srv_refuses_gives_its_reason() {
    for (reply, wanted) in [(json!({ "error": "bad manifest" }), "the widget wasn't installed: bad manifest"), (json!("boom"), "the widget wasn't installed: unknown error")] {
        let srv = stub_srv("400 Bad Request", reply);
        let fx = Fixture::new(&srv);
        let err = call("WidgetInstall", &json!({ "path": absolute("w") }), &fx.cx()).await.unwrap_err().to_string();
        assert_eq!(err, wanted);
    }
}

#[tokio::test]
async fn opening_a_view_splits_beside_the_agent_with_the_widgets_defaults_and_the_agents_own_meta() {
    let srv = stub_srv("200 OK", widgets("approved"));
    let fx = Fixture::new(&srv);
    let said = call("OpenWidget", &json!({ "view": "ext:a.b/main", "meta": { "k": "v" }, "split": "down" }), &fx.cx()).await.unwrap();
    assert_eq!(said, "Opened ext:a.b/main (pane b9).");
    let seen = srv.seen.lock().unwrap().clone();
    assert_eq!(seen.iter().map(|(p, _)| p.as_str()).collect::<Vec<_>>(), ["/api/v1/widgets", "/api/v1/pane/open"]);
    assert_eq!(
        seen[1].1,
        json!({
            "view": "ext:a.b/main", "focus": true, "split_direction": "down", "split_reference_block_id": "blk",
            "meta": { "d": 1, "widget:a.b:k": "v", "view": "ext:a.b/main" }
        })
    );
    // Any other split is to the right; an agent with no block opens nowhere in particular.
    call("OpenWidget", &json!({ "view": "ext:a.b/main", "split": "left" }), &fx.cx()).await.unwrap();
    assert_eq!(srv.seen.lock().unwrap()[3].1["split_direction"], "right");
    let no_block = ToolCtx { block_id: "", ..fx.cx() };
    call("OpenWidget", &json!({ "view": "ext:a.b/main" }), &no_block).await.unwrap();
    let last = srv.seen.lock().unwrap()[5].1.clone();
    assert!(last.get("split_direction").is_none() && last.get("split_reference_block_id").is_none(), "{last}");
}

#[tokio::test]
async fn a_view_that_cannot_be_opened_says_why() {
    let srv = stub_srv("200 OK", widgets("pending"));
    let fx = Fixture::new(&srv);
    let err = call("OpenWidget", &json!({ "view": "ext:a.b/main" }), &fx.cx()).await.unwrap_err().to_string();
    assert_eq!(err, "a.b is pending, so it can't open; the user approves or turns it on in Settings → Widgets");
    let err = call("OpenWidget", &json!({ "view": "ext:x/y" }), &fx.cx()).await.unwrap_err().to_string();
    assert_eq!(err, "no installed widget has the view ext:x/y; Widget action \"list\" shows the views there are");
    drop(fx);
    let mut stateless = widgets("approved");
    stateless["packages"][0].as_object_mut().unwrap().remove("state");
    let srv = stub_srv("200 OK", stateless);
    let fx = Fixture::new(&srv);
    let err = call("OpenWidget", &json!({ "view": "ext:a.b/main" }), &fx.cx()).await.unwrap_err().to_string();
    assert!(err.starts_with("a.b is not approved, so it can't open"), "{err}");
    // Nothing was opened for any of them.
    assert!(srv.seen.lock().unwrap().iter().all(|(p, _)| p != "/api/v1/pane/open"));
}

#[tokio::test]
async fn a_tool_missing_what_it_needs_refuses_before_it_asks_srv() {
    let srv = stub_srv("200 OK", json!({}));
    let fx = Fixture::new(&srv);
    let no_url = ToolCtx { local_url: "", ..fx.cx() };
    let no_key = ToolCtx { auth_key: "", ..fx.cx() };
    let unset = "AGENTMUX_LOCAL_URL and AGENTMUX_AUTH_KEY must be set";
    let refusals = [
        ("WidgetInstall", json!({}), fx.cx(), "missing required parameter: path"),
        ("WidgetInstall", json!({ "path": "" }), fx.cx(), "missing required parameter: path"),
        ("OpenWidget", json!({}), fx.cx(), "missing required parameter: view"),
        ("WidgetList", json!({}), no_url, unset),
        ("WidgetList", json!({}), no_key, unset),
        ("WidgetInstall", json!({ "path": "/p" }), no_url, unset),
        ("WidgetInstall", json!({ "path": "/p" }), no_key, unset),
        ("OpenWidget", json!({ "view": "v" }), no_url, unset),
        ("OpenWidget", json!({ "view": "v" }), no_key, unset),
        // The environment is checked before the arguments.
        ("OpenWidget", json!({}), no_url, unset),
    ];
    for (tool, args, cx, wanted) in refusals {
        let err = call(tool, &args, &cx).await.expect_err(&format!("{tool} {args} should refuse")).to_string();
        assert!(err.contains(wanted), "{tool} {args}: {err:?}, wanted {wanted:?}");
    }
    assert!(srv.seen.lock().unwrap().is_empty(), "a refused call asked srv");
}

#[tokio::test]
async fn a_list_srv_refuses_names_the_status() {
    for (tool, args) in [("WidgetList", json!({})), ("OpenWidget", json!({ "view": "v" }))] {
        let srv = stub_srv("500 Internal Server Error", json!("boom"));
        let fx = Fixture::new(&srv);
        let err = call(tool, &args, &fx.cx()).await.unwrap_err().to_string();
        assert_eq!(err, "listing widgets failed: HTTP 500 Internal Server Error: \"boom\"", "{tool}");
    }
}

#[tokio::test]
async fn a_name_that_is_not_a_widget_tool_is_not_in_the_family() {
    let srv = stub_srv("200 OK", json!({}));
    let fx = Fixture::new(&srv);
    assert!(call("WidgetNope", &json!({}), &fx.cx()).await.is_err());
    assert!(srv.seen.lock().unwrap().is_empty());
}

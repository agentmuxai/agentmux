// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::*;

fn filter(v: Value) -> Map<String, Value> {
    v.as_object().unwrap().clone()
}

fn api_node(data: Value) -> FlowNode {
    FlowNode { id: "poll".to_string(), position: Default::default(), data, node_type: None }
}

#[test]
fn the_items_are_the_list_at_the_path_each_with_the_id_its_own_path_gives() {
    let body = json!({ "workflow_runs": [
        { "id": 11, "conclusion": "failure", "head": { "sha": "abc" } },
        { "id": "12", "conclusion": "success" },
        { "conclusion": "no id" },
        { "id": "", "conclusion": "empty id" },
        { "id": 11, "conclusion": "again" },
    ] });
    let got = items_in(&body, "workflow_runs", "", 100).unwrap();
    let ids: Vec<&str> = got["items"].as_array().unwrap().iter().map(|i| i["id"].as_str().unwrap()).collect();
    // A number is text, an item with no usable id is not one; the id is the item's own, whatever path says it.
    assert_eq!(ids, ["11", "12", "11"], "the list is the answer's own order; a repeated id is dropped later, when the poll reads it");
    assert_eq!(got["items"][0]["item"]["head"]["sha"], "abc", "the item is the answer's own, whole");
    let by_sha = items_in(&json!([{ "head": { "sha": "a1" } }, { "head": { "sha": "b2" } }]), "", "head.sha", 100).unwrap();
    assert_eq!(by_sha["items"][1]["id"], "b2", "the answer may itself be the list, and the id a dotted path");
    assert_eq!(items_in(&json!({ "a": [{ "id": 1 }, { "id": 2 }, { "id": 3 }] }), "a", "id", 2).unwrap()["items"].as_array().unwrap().len(), 2, "limit cuts the list");
    assert_eq!(items_in(&json!([{ "id": 1 }]), "", "id", 0).unwrap()["items"].as_array().unwrap().len(), 1, "a limit of 0 still keeps one: the filter's 1 to 200 is checked where it is written");
}

#[test]
fn an_answer_that_is_not_a_list_says_where_it_looked() {
    assert!(items_in(&json!({ "a": 1 }), "", "id", 10).unwrap_err().contains("give `items`"));
    assert!(items_in(&json!({ "a": 1 }), "data.items", "id", 10).unwrap_err().contains("no list at `data.items`"));
    assert!(items_in(&json!({ "data": { "items": "x" } }), "data.items", "id", 10).is_err());
}

#[test]
fn only_a_2xx_answer_is_a_list_and_a_redirect_says_a_credential_follows_none() {
    let ok = json!({ "status": 200, "body": { "items": [{ "id": 1 }] }, "headers": {} });
    assert_eq!(items_of_output(&ok, "items", "id", 10).unwrap()["items"][0]["id"], "1");
    let as_text = json!({ "status": 200, "body": "[{\"id\": 7}]", "headers": {} });
    assert_eq!(items_of_output(&as_text, "", "id", 10).unwrap()["items"][0]["id"], "7");
    assert!(items_of_output(&json!({ "status": 200, "body": "<html>", "headers": {} }), "", "id", 10).unwrap_err().contains("not JSON"));
    let moved = items_of_output(&json!({ "status": 302, "body": "", "headers": { "location": "https://elsewhere.example.net/x" } }), "", "id", 10).unwrap_err();
    assert!(moved.contains("302") && moved.contains("follows no redirect"), "{moved}");
    let missing = items_of_output(&json!({ "status": 404, "body": {}, "headers": {} }), "", "id", 10).unwrap_err();
    assert_eq!(missing, "the site answered 404");
    let denied = items_of_output(&json!({ "status": 401, "body": {}, "headers": {} }), "", "id", 10).unwrap_err();
    assert!(!denied.contains("redirect"), "{denied}");
}

/// A poll is the API block's own call: every refusal the block makes of a credentialed request, the poll makes in the same
/// words, and for the same reasons, because it runs the same code.
#[tokio::test]
async fn a_poll_is_refused_exactly_as_the_api_block_refuses_the_same_request() {
    let auth = json!({ "vault": "api-key-acme", "as": "bearer" });
    let cases = [
        json!({ "url": "http://api.example.com/x", "auth": auth }),
        json!({ "url": "https://{{trigger.host}}/x", "auth": auth }),
        json!({ "url": "https://api.example.com/x", "auth": { "vault": "k", "as": "basic" } }),
        json!({ "url": "https://api.example.com/x", "headers": { "Authorization": "Bearer mine" }, "auth": auth }),
        json!({ "url": "https://api.example.com/x", "headers": { "host": "other-tenant.example.net" }, "auth": auth }),
        json!({ "url": "https://user:pw@api.example.com/x", "auth": auth }),
    ];
    for case in cases {
        let mut asked = filter(case.clone());
        asked.insert("query".into(), json!("api"));
        asked.insert("drone_id".into(), json!("nightly"));
        let polled = ask(asked).await.unwrap_err();
        let mut data = case.clone();
        data["method"] = json!("GET");
        let mut scope = ExecutionScope::new();
        scope.drone_id = "nightly".into();
        scope.never_ask = true;
        let direct = super::super::api::run(&api_node(data), &scope).await.unwrap_err();
        assert_eq!(polled, direct, "the poll and the API block refuse {case} in the same words");
        assert!(!polled.contains("sk_"), "{polled}");
    }
}

#[tokio::test]
async fn a_poll_with_a_credential_belongs_to_a_drone_and_one_with_no_url_is_nothing() {
    let none = ask(filter(json!({ "query": "api" }))).await.unwrap_err();
    assert!(none.contains("has a url"), "{none}");
    let no_drone = ask(filter(json!({ "query": "api", "url": "https://api.example.com/x", "auth": { "vault": "api-key-acme" } }))).await.unwrap_err();
    assert!(no_drone.contains("belongs to a drone job"), "{no_drone}");
}

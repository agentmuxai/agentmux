// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What an agent asks for when it names a route, resolved once for `Agents spawn`, `create` and `update`,
//! so the three cannot disagree (spec section 5.1). Pure: the key check is a closure the caller supplies.

use super::catalog::{self, Model, Route};
use super::launch::Choice;
use serde_json::{json, Value};

/// The words that mean the harness's own sign-in rather than a route.
const OWN_LOGIN: &[&str] = &["anthropic", "login", "default"];

#[derive(Debug)]
pub struct Picked {
    pub route: &'static Route,
    pub model: &'static Model,
}

/// The route and model a new agent runs on.
///
/// - `route` names one: it must serve the harness, and `model` (or the route's default) must be one it serves.
/// - `route` is the harness's own sign-in: no route, whatever the leader had.
/// - `route` is absent: the leader's route is inherited, and `model` picks a model on it.
/// - Neither: no route; `model` is the harness's own model choice, which the caller handles.
///
/// A picked route needs its key in the Vault; when it is not there the answer says what to ask for.
pub fn resolve(harness: &str, route: Option<&str>, model: Option<&str>, inherited: Option<&Choice>, key_present: impl Fn(&Route) -> bool) -> Result<Option<Picked>, String> {
    let route = route.map(str::trim).filter(|r| !r.is_empty());
    let model = model.map(str::trim).filter(|m| !m.is_empty());
    let (route_id, model_id) = match (route, inherited) {
        (Some(r), _) if OWN_LOGIN.contains(&r.to_lowercase().as_str()) => return Ok(None),
        (Some(r), _) => (r.to_string(), model.unwrap_or("").to_string()),
        (None, Some(c)) => (c.route.clone(), model.unwrap_or(&c.model).to_string()),
        (None, None) => return Ok(None),
    };
    let (route, model) = catalog::validate(harness, &route_id, &model_id)?;
    if !key_present(route) {
        return Err(format!(
            "KEY_MISSING: route {} needs a Vault api-key entry labelled \"{}\", and the Vault has none. \
             Ask the person to add it (Vault request, kind api-key, that label), then try again",
            route.id, route.key_label
        ));
    }
    Ok(Some(Picked { route, model }))
}

/// The line the person's approval names: where the agent's model traffic goes and who pays.
pub fn consent_line(picked: &Picked) -> String {
    format!(
        " It runs {} on {} through the \"{}\" route, billed to the \"{}\" key in the Vault, not to a plan; \
         that costs money for every turn until it is stopped.",
        picked.route.harness, picked.model.label, picked.route.label, picked.route.key_label
    )
}

/// `Agents routes`: the catalog as a caller may use it. `key_present` says whether a route's key is in the Vault.
pub fn listing(harness: Option<&str>, key_present: impl Fn(&Route) -> bool) -> Value {
    let routes: Vec<Value> = catalog::ROUTES
        .iter()
        .filter(|r| harness.is_none_or(|h| r.harness == h))
        .map(|r| {
            let models: Vec<Value> = r
                .models
                .iter()
                .map(|m| {
                    json!({
                        "id": m.id, "label": m.label, "context": m.context, "tools": m.tools, "thinking": m.thinking, "images": m.images,
                        "price_in_per_million": m.price_in, "price_out_per_million": m.price_out, "default": m.id == r.default_model,
                    })
                })
                .collect();
            json!({ "route": r.id, "label": r.label, "harness": r.harness, "paid": true, "key_label": r.key_label, "key_present": key_present(r), "models": models })
        })
        .collect();
    json!({
        "routes": routes,
        "note": "pass route (and optionally model) to Agents spawn or create; route \"anthropic\" is the harness's own sign-in. A route is billed to its key, not to a plan",
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn has_key(_: &Route) -> bool {
        true
    }

    #[test]
    fn a_named_route_gets_its_default_model_or_the_one_asked_for() {
        let p = resolve("claude", Some("openrouter"), None, None, has_key).unwrap().unwrap();
        assert_eq!((p.route.id, p.model.id), ("openrouter", "deepseek/deepseek-v4.1-flash-1017"));
        let p = resolve("claude", Some("openrouter"), Some("deepseek/deepseek-v4-pro-0813"), None, has_key).unwrap().unwrap();
        assert_eq!(p.model.id, "deepseek/deepseek-v4-pro-0813");
    }

    #[test]
    fn the_harness_own_login_means_no_route_even_when_the_leader_has_one() {
        let leader = Choice { route: "openrouter".into(), model: "deepseek/deepseek-v4.1-flash-1017".into() };
        for word in ["anthropic", "Login", "default"] {
            assert!(resolve("claude", Some(word), Some("sonnet"), Some(&leader), has_key).unwrap().is_none(), "{word}");
        }
    }

    #[test]
    fn a_worker_inherits_its_leaders_route_and_a_model_changes_only_the_model() {
        let leader = Choice { route: "openrouter".into(), model: "deepseek/deepseek-v4.1-flash-1017".into() };
        let same = resolve("claude", None, None, Some(&leader), has_key).unwrap().unwrap();
        assert_eq!(same.model.id, "deepseek/deepseek-v4.1-flash-1017");
        let other = resolve("claude", None, Some("deepseek/deepseek-v4-pro-0813"), Some(&leader), has_key).unwrap().unwrap();
        assert_eq!((other.route.id, other.model.id), ("openrouter", "deepseek/deepseek-v4-pro-0813"));
        assert!(resolve("claude", None, Some("sonnet"), None, has_key).unwrap().is_none(), "no route: the model is the harness's own");
    }

    #[test]
    fn a_route_that_cannot_be_used_says_why() {
        assert!(resolve("claude", Some("nowhere"), None, None, has_key).unwrap_err().contains("openrouter"));
        assert!(resolve("codex", Some("openrouter"), None, None, has_key).unwrap_err().contains("serves claude"));
        let missing = resolve("claude", Some("openrouter"), None, None, |_| false).unwrap_err();
        assert!(missing.starts_with("KEY_MISSING") && missing.contains("OpenRouter API key"), "{missing}");
    }

    #[test]
    fn the_approval_names_the_route_the_model_and_who_pays() {
        let p = resolve("claude", Some("openrouter"), None, None, has_key).unwrap().unwrap();
        let line = consent_line(&p);
        assert!(line.contains("DeepSeek V4.1 Flash") && line.contains("OpenRouter") && line.contains("not to a plan"), "{line}");
    }

    #[test]
    fn the_listing_says_which_routes_have_a_key_and_what_they_cost() {
        let all = listing(None, |_| false);
        assert_eq!(all["routes"][0]["route"], "openrouter");
        assert_eq!(all["routes"][0]["key_present"], false);
        assert_eq!(all["routes"][0]["models"][0]["default"], true);
        assert!(all["routes"][0]["models"][0]["price_in_per_million"].as_f64().unwrap() > 0.0);
        assert!(listing(Some("codex"), has_key)["routes"].as_array().unwrap().is_empty());
    }
}

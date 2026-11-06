// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The routes AgentMux ships. A route's address is fixed here and never read from an agent's request.
//!
//! The model list and prices were read from OpenRouter's public models API on 2026-11-06. They are
//! the values the gateway counts spend with, so a price that moves upstream is a change here.

/// One model a route serves. Prices are US dollars per million tokens.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Model {
    pub id: &'static str,
    pub label: &'static str,
    pub context: u32,
    pub tools: bool,
    pub thinking: bool,
    pub images: bool,
    pub price_in: f64,
    pub price_out: f64,
    pub price_cache_read: f64,
}

/// A way to reach models for one harness.
#[derive(Debug, Clone, Copy)]
pub struct Route {
    pub id: &'static str,
    pub label: &'static str,
    /// The harness it serves; its protocol decides the paths the gateway forwards.
    pub harness: &'static str,
    /// Where the gateway forwards to, with no trailing slash.
    pub base_url: &'static str,
    /// The start of the label of the Vault `api-key` entry that holds the route's key.
    pub key_label: &'static str,
    pub models: &'static [Model],
    pub default_model: &'static str,
}

const OPENROUTER_MODELS: &[Model] = &[
    Model {
        id: "deepseek/deepseek-v4.1-flash-1017",
        label: "DeepSeek V4.1 Flash",
        context: 1_048_576,
        tools: true,
        thinking: true,
        images: true,
        price_in: 0.15,
        price_out: 0.60,
        price_cache_read: 0.003,
    },
    Model {
        id: "deepseek/deepseek-v4-pro-0813",
        label: "DeepSeek V4 Pro",
        context: 1_048_576,
        tools: true,
        thinking: true,
        images: false,
        price_in: 0.1165,
        price_out: 4.66,
        price_cache_read: 0.111,
    },
];

pub const ROUTES: &[Route] = &[Route {
    id: "openrouter",
    label: "OpenRouter",
    harness: "claude",
    base_url: "https://openrouter.ai/api",
    key_label: "OpenRouter API key",
    models: OPENROUTER_MODELS,
    default_model: "deepseek/deepseek-v4.1-flash-1017",
}];

/// The route with this id.
pub fn route(id: &str) -> Option<&'static Route> {
    ROUTES.iter().find(|r| r.id == id)
}

/// A test's stand-in upstream, in place of the route's own address.
#[cfg(test)]
pub(crate) static TEST_BASE_URL: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

impl Route {
    /// Where the gateway forwards to: the route's own address, fixed in this file.
    pub fn base(&self) -> String {
        #[cfg(test)]
        if let Some(url) = TEST_BASE_URL.lock().unwrap().clone() {
            return url;
        }
        self.base_url.to_string()
    }

    pub fn model(&self, id: &str) -> Option<&'static Model> {
        let id = id.trim().trim_end_matches("[1m]");
        self.models.iter().find(|m| m.id == id)
    }
}

/// The one validation spawn, create, update and the form share: the route must exist and serve the
/// harness, and the model must be one the route serves (empty means the route's default).
pub fn validate(harness: &str, route_id: &str, model: &str) -> Result<(&'static Route, &'static Model), String> {
    let route = route(route_id).ok_or_else(|| {
        let known: Vec<&str> = ROUTES.iter().map(|r| r.id).collect();
        format!("unknown model route {route_id:?}; the routes are {}", known.join(", "))
    })?;
    if route.harness != harness {
        return Err(format!("route {} serves {}, not {harness}", route.id, route.harness));
    }
    let wanted = if model.trim().is_empty() { route.default_model } else { model };
    let model = route.model(wanted).ok_or_else(|| {
        let known: Vec<&str> = route.models.iter().map(|m| m.id).collect();
        format!("route {} does not serve {wanted:?}; it serves {}", route.id, known.join(", "))
    })?;
    Ok((route, model))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_model_of_every_route_is_one_it_serves() {
        for r in ROUTES {
            assert!(r.model(r.default_model).is_some(), "{}", r.id);
        }
    }

    #[test]
    fn validation_names_what_is_wrong() {
        assert!(validate("claude", "openrouter", "").is_ok());
        assert!(validate("claude", "openrouter", "deepseek/deepseek-v4-pro-0813[1m]").is_ok());
        assert!(validate("claude", "nowhere", "").unwrap_err().contains("openrouter"));
        assert!(validate("codex", "openrouter", "").unwrap_err().contains("serves claude"));
        assert!(validate("claude", "openrouter", "gpt-5").unwrap_err().contains("deepseek/deepseek-v4.1-flash-1017"));
    }
}

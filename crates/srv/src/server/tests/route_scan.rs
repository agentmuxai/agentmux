// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The routes `routes.rs` registers, read from its source, for the tests that compare a list with
//! the router (`http_action_tests`, `backend::route_audience`).

/// One `.route(path, handlers)` call: the path as written (`:param` and `*rest` kept) and the
/// text after it, which names the methods.
pub(crate) struct RegisteredRoute {
    pub path: String,
    pub methods: String,
}

/// Every route in `routes.rs`, comments skipped.
pub(crate) fn registered_routes() -> Vec<RegisteredRoute> {
    let source: String = include_str!("../routes.rs")
        .lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n");
    let mut found = Vec::new();
    let mut rest = source.as_str();
    while let Some(at) = rest.find(".route(") {
        let body_start = at + ".route(".len();
        let mut depth = 1;
        let mut end = body_start;
        for (i, c) in rest[body_start..].char_indices() {
            match c {
                '(' => depth += 1,
                ')' => depth -= 1,
                _ => {}
            }
            if depth == 0 {
                end = body_start + i;
                break;
            }
        }
        let body = &rest[body_start..end];
        let (path, methods) = match body.trim_start().strip_prefix('"') {
            Some(quoted) => {
                let close = quoted.find('"').expect("a closing quote");
                (quoted[..close].to_string(), quoted[close..].to_string())
            }
            None => {
                // The one route whose path is a named constant.
                assert!(body.contains("STOP_RUN_PATH"), "a route path that is neither a literal nor STOP_RUN_PATH: {body}");
                (crate::server::jobs_remote_stop::STOP_RUN_PATH.to_string(), body.to_string())
            }
        };
        found.push(RegisteredRoute { path, methods });
        rest = &rest[end..];
    }
    found
}

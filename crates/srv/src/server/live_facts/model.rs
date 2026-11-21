// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Which model an agent runs on, and where that answer came from (spec §3 and §7). Pure: the caller reads
//! the runtime state and hands it in, so every order of preference is a test.
//!
//! In order: the model the agent's route sends to; what the CLI itself reported using (`effective`);
//! the model of its last call (`observed`); what its process was started with, then what its pane
//! remembers asking for (`requested`); and otherwise `unknown`, with why.

use serde::Serialize;

use crate::backend::agent_runtime::AgentRuntimeEvent;
use crate::backend::model_label::label_of;

/// Where a model answer came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
#[serde(rename_all = "snake_case")]
pub enum ModelSource {
    /// The CLI reported it (`get_settings`, read after every turn).
    Effective,
    /// The model of the last call the agent made.
    Observed,
    /// The agent's model route: the gateway sends its requests to this model.
    Route,
    /// What the process was started with or the pane remembers asking for: not yet confirmed by the CLI.
    Requested,
    /// Nothing says.
    Unknown,
}

/// The model route an agent is on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct RouteFacts {
    pub id: String,
    pub label: String,
}

/// The model and effort an agent runs on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct ModelFacts {
    /// The model id, as its harness names it.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub id: Option<String>,
    /// A short name for it ("Sonnet 5.5"); the id itself when there is no shorter name.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub label: Option<String>,
    pub source: ModelSource,
    /// Why there is no model, when `source` is `unknown`.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub why: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub effort: Option<String>,
    /// A model or effort change is waiting for the end of the turn, so the model shown is not yet the new one.
    pub pending_switch: bool,
    /// The short name of the model the waiting change will run, when the change is to another model (not to another effort).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub pending_label: Option<String>,
    /// The short name of the model its definition asks for, when that is not the one it runs.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub definition_label: Option<String>,
    /// The route the agent is on, when it is on one.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub route: Option<RouteFacts>,
    /// The model it runs is not the one its definition asks for (a switch from the menu, a route, a resume).
    pub differs_from_definition: bool,
}

/// What the caller knows about one agent.
#[derive(Default)]
pub struct Inputs<'a> {
    /// The latest `agentruntime` event of its pane.
    pub event: Option<&'a AgentRuntimeEvent>,
    /// The model of the last call its pane made (`context_reading`).
    pub observed: Option<&'a str>,
    /// The route it is on: the route's id and label, the model's id and label.
    pub route: Option<(&'a str, &'a str, &'a str, &'a str)>,
    /// What the pane's `agent:runtime` or `agent:model` meta says it asked for.
    pub meta_model: Option<&'a str>,
    pub meta_effort: Option<&'a str>,
    /// The `--model` of its definition's flags.
    pub definition_model: Option<&'a str>,
    /// A process is alive in it.
    pub running: bool,
}

/// Two names for one model: equal, or one is the other's alias (`sonnet` and `claude-sonnet-5-5`).
pub fn same_model(a: &str, b: &str) -> bool {
    let norm = |s: &str| s.trim().trim_end_matches("[1m]").to_ascii_lowercase();
    let (a, b) = (norm(a), norm(b));
    !a.is_empty() && !b.is_empty() && (a == b || a.contains(&b) || b.contains(&a))
}

/// The model and effort of an agent, and where they came from.
pub fn resolve(i: &Inputs) -> ModelFacts {
    let live = i.event.filter(|e| e.running);
    let (id, source) = if let Some((_, _, model, _)) = i.route {
        (Some(model.to_string()), ModelSource::Route)
    } else if let Some(m) = live.and_then(|e| e.effective_model.clone()) {
        (Some(m), ModelSource::Effective)
    } else if let Some(m) = i.observed {
        (Some(m.to_string()), ModelSource::Observed)
    } else if let Some(m) = live.and_then(|e| e.model.clone()).or_else(|| i.meta_model.map(str::to_string)) {
        (Some(m), ModelSource::Requested)
    } else {
        (None, ModelSource::Unknown)
    };
    let label = match (i.route, &id) {
        (Some((_, _, _, model_label)), _) => Some(model_label.to_string()),
        (None, Some(m)) => Some(label_of(m)),
        _ => None,
    };
    let why = (source == ModelSource::Unknown).then(|| {
        if i.running { "its harness reports no model and none was asked for" } else { "no process is running and no model is on record" }.to_string()
    });
    let effort = live
        .and_then(|e| e.effective_effort.clone().or_else(|| e.effort.clone()))
        .or_else(|| i.meta_effort.map(str::to_string));
    let differs = source != ModelSource::Route
        && matches!((&id, i.definition_model), (Some(now), Some(asked)) if !same_model(now, asked));
    let pending_label = i
        .event
        .filter(|e| e.restart_pending)
        .and_then(|e| e.model.as_deref())
        .filter(|asked| id.as_deref().is_none_or(|now| !same_model(now, asked)))
        .map(label_of);
    let definition_label = i.definition_model.filter(|_| differs).map(label_of);
    ModelFacts {
        id,
        label,
        source,
        why,
        effort,
        pending_label,
        definition_label,
        pending_switch: i.event.is_some_and(|e| e.restart_pending),
        route: i.route.map(|(id, label, _, _)| RouteFacts { id: id.to_string(), label: label.to_string() }),
        differs_from_definition: differs,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::agent_runtime::agent_runtime_event;

    fn event(running: bool, model: Option<&str>, effective: Option<&str>, restart: bool) -> AgentRuntimeEvent {
        let mut e = agent_runtime_event("b1", None, restart, None);
        e.running = running;
        e.model = model.map(str::to_string);
        e.effective_model = effective.map(str::to_string);
        e
    }

    #[test]
    fn what_the_cli_reported_beats_what_was_asked_for_and_says_so() {
        let e = event(true, Some("sonnet"), Some("claude-opus-5-5"), false);
        let m = resolve(&Inputs { event: Some(&e), observed: Some("claude-haiku-5-5"), running: true, ..Default::default() });
        assert_eq!((m.id.as_deref(), m.label.as_deref(), m.source), (Some("claude-opus-5-5"), Some("Opus 5.5"), ModelSource::Effective));
    }

    #[test]
    fn without_a_report_the_last_call_then_the_start_then_the_pane_s_memory_answer_in_that_order() {
        let e = event(true, Some("sonnet"), None, false);
        let observed = resolve(&Inputs { event: Some(&e), observed: Some("claude-sonnet-5-5"), running: true, ..Default::default() });
        assert_eq!((observed.source, observed.id.as_deref()), (ModelSource::Observed, Some("claude-sonnet-5-5")));
        let asked = resolve(&Inputs { event: Some(&e), running: true, ..Default::default() });
        assert_eq!((asked.source, asked.label.as_deref()), (ModelSource::Requested, Some("Sonnet")));
        let remembered = resolve(&Inputs { meta_model: Some("claude-opus-5-5"), ..Default::default() });
        assert_eq!((remembered.source, remembered.label.as_deref()), (ModelSource::Requested, Some("Opus 5.5")));
    }

    #[test]
    fn an_event_of_a_stopped_pane_reports_nothing_it_only_remembers() {
        let stopped = event(false, Some("sonnet"), Some("claude-opus-5-5"), false);
        let m = resolve(&Inputs { event: Some(&stopped), ..Default::default() });
        assert_eq!((m.source, m.id.as_deref()), (ModelSource::Unknown, None));
        assert!(m.why.unwrap().contains("no process"));
        let alive = resolve(&Inputs { running: true, ..Default::default() });
        assert!(alive.why.unwrap().contains("harness reports no model"));
    }

    #[test]
    fn a_route_names_the_model_its_gateway_sends_to_and_is_never_a_difference() {
        let e = event(true, Some("sonnet"), Some("claude-sonnet-5-5"), false);
        let m = resolve(&Inputs {
            event: Some(&e),
            route: Some(("example", "Example route", "example-large", "Example Large")),
            definition_model: Some("opus"),
            running: true,
            ..Default::default()
        });
        assert_eq!((m.source, m.id.as_deref(), m.label.as_deref()), (ModelSource::Route, Some("example-large"), Some("Example Large")));
        assert_eq!(m.route, Some(RouteFacts { id: "example".into(), label: "Example route".into() }));
        assert!(!m.differs_from_definition);
    }

    #[test]
    fn a_model_that_is_not_the_definitions_is_marked_and_an_alias_of_it_is_not() {
        let e = event(true, None, Some("claude-opus-5-5"), false);
        let differs = resolve(&Inputs { event: Some(&e), definition_model: Some("sonnet"), running: true, ..Default::default() });
        assert!(differs.differs_from_definition);
        let same = resolve(&Inputs { event: Some(&e), definition_model: Some("opus"), running: true, ..Default::default() });
        assert!(!same.differs_from_definition, "`opus` is the definition's name for claude-opus-5-5");
        let none = resolve(&Inputs { event: Some(&e), running: true, ..Default::default() });
        assert!(!none.differs_from_definition, "a definition that asks for nothing cannot differ");
    }

    #[test]
    fn a_switch_waiting_for_the_end_of_the_turn_is_flagged_and_the_effort_follows_the_report() {
        let mut e = event(true, Some("opus"), Some("claude-sonnet-5-5"), true);
        e.effective_effort = Some("high".to_string());
        e.effort = Some("low".to_string());
        let m = resolve(&Inputs { event: Some(&e), running: true, ..Default::default() });
        assert!(m.pending_switch);
        assert_eq!(m.pending_label.as_deref(), Some("Opus"), "the model the change will run, by its short name");
        let effort_only = event(true, Some("sonnet"), Some("claude-sonnet-5-5"), true);
        let same = resolve(&Inputs { event: Some(&effort_only), running: true, ..Default::default() });
        assert!(same.pending_switch && same.pending_label.is_none(), "an effort change names no model");
        let named = resolve(&Inputs { event: Some(&e), definition_model: Some("haiku"), running: true, ..Default::default() });
        assert_eq!(named.definition_label.as_deref(), Some("Haiku"));
        assert_eq!(m.definition_label, None);
        assert_eq!(m.effort.as_deref(), Some("high"), "the effective effort wins over the requested one");
        let remembered = resolve(&Inputs { meta_effort: Some("medium"), ..Default::default() });
        assert_eq!(remembered.effort.as_deref(), Some("medium"));
    }
}

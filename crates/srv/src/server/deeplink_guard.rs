// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! srv's guard of an agent's deep-link open (docs/specs/SPEC_DEEP_LINKS_ONE_FRAMEWORK_2026_11_20.md §4.2, R16, G1 to
//! G3, G5, G7, G9, G12). `POST /api/v1/pane/open_link` reads the link with the shared parser
//! (`agentmux_common::deeplinks`), applies the rules an agent's open is held to, records the open as the agent, and
//! hands what it let through to the window (`sagas::deeplink_open`), whose executor applies the rules only the window
//! can (R5, the pane caps and the typing queue of G3, G6).
//!
//! The rules here, each with its test below:
//! - Only an agent, known by its token, calls this route: a person's click goes to the app's executor directly.
//! - G1, G5: a view an agent may not open is refused; a place marked as the caller's own must name the caller; the
//!   Audit's sound and message rows, whose owner srv can't tell, are refused in build 1; a container agent gets no
//!   path destination (its files open through `Pane` open's snapshot).
//! - G7: a path is canonicalised once, the structural checks run again on the result, and the open uses that string.
//!   A network path, one that does not exist and one of the wrong kind (a folder for the Editor) are refused.
//! - G3: a window target is refused; a tab or pane outside the caller's own tab lands in its own tab instead; ten
//!   opens a minute per agent, refused (not queued) over it.
//! - G9: every open is recorded as the agent, refused and queued ones too, with the outcome and the reason.
//! - G12: everything is checked now, when the open happens.

use std::collections::{HashMap, VecDeque};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use agentmux_common::deeplinks::{self, AgentRule, LinkError, ParsedLink, Segment, Sensitivity, Value};
use serde::Deserialize;

use super::*;
use crate::sagas::deeplink_open::{self, DeepLinkOutcome};

/// Opens an agent may ask for in [`RATE_WINDOW`] (G3, D3).
pub(crate) const RATE_LIMIT: usize = 10;
pub(crate) const RATE_WINDOW: Duration = Duration::from_secs(60);

/// The body of `POST /api/v1/pane/open_link`.
#[derive(Debug, Clone, Default, Deserialize)]
pub(crate) struct OpenLinkRequest {
    pub link: String,
    /// `beside` (the default), `new`, `tab:<tab id>`, `pane:<block id>` or `window:<window id>` (R2).
    #[serde(default)]
    pub target: Option<String>,
}

/// Who asks, as srv knows it: never from the request's words.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct CallerFacts {
    /// The agent's id (its slug).
    pub agent_id: String,
    pub container: bool,
    pub block_id: Option<String>,
    pub tab_id: Option<String>,
    pub window_id: Option<String>,
}

/// A refusal: a reason code of the executor's vocabulary (`open-types.ts`), the sentence, and what would work.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Refusal {
    pub code: &'static str,
    pub text: String,
    pub remedy: Option<String>,
}

fn refusal(code: &'static str, text: impl Into<String>, remedy: Option<String>) -> Refusal {
    Refusal { code, text: text.into(), remedy }
}

impl From<LinkError> for Refusal {
    fn from(e: LinkError) -> Self {
        let code = if e == LinkError::NotYet { "not_yet" } else { "invalid_link" };
        refusal(code, deeplinks::REGISTRY.reason(e.as_str()), None)
    }
}

/// The words for where a link goes, in English: srv has no i18n of its own, so it names the view and place by id.
pub(crate) fn label_of(link: &ParsedLink) -> String {
    let view = link.view.id;
    match link.place {
        None => view.to_string(),
        Some(place) => {
            let held = place.segments.iter().find_map(|s| match s {
                Segment::Var(v) if v.kind == deeplinks::Kind::Enum => link.values.get(v.name).map(Value::to_string),
                _ => None,
            });
            format!("{view} {} {}", deeplinks::REGISTRY.separator, held.unwrap_or_else(|| place.id.to_string()))
        }
    }
}

/// The values and parameters a place marks as the caller's own (G5).
fn own_values<'a>(link: &'a ParsedLink) -> Vec<(&'static str, Option<&'a Value>)> {
    let mut out = Vec::new();
    if let Some(place) = link.place {
        for s in place.segments {
            if let Segment::Var(v) = s {
                if v.agent_own {
                    out.push((v.name, link.values.get(v.name)));
                }
            }
        }
        for p in place.params {
            if p.agent_own && p.only_with.is_none_or(|(var, allowed)| link.values.get(var).is_some_and(|h| allowed.contains(&h.to_string().as_str()))) {
                out.push((p.name, link.params.get(p.name)));
            }
        }
    }
    out
}

/// G1 and G5: whether this agent may open `link`. May fill in the caller's own id where the place takes one and the
/// link left it out, so a place that would show "whoever is selected" shows the caller's own.
pub(crate) fn check_rules(link: &mut ParsedLink, who: &CallerFacts) -> Result<(), Refusal> {
    let view = link.view;
    match view.agent {
        AgentRule::No => return Err(refusal("not_allowed", format!("an agent can't open {}", view.id), None)),
        AgentRule::Approved | AgentRule::Reveal => {
            // Declared for a later build: the parser refuses them as not_yet before this.
            return Err(refusal("not_yet", deeplinks::REGISTRY.reason("not_yet"), None));
        }
        AgentRule::Own => {
            let own_link = format!("agentmux://{}/agent/{}", view.id, who.agent_id);
            match link.place.map(|p| p.id) {
                Some("agent") => {}
                _ => {
                    return Err(refusal(
                        "not_allowed",
                        format!("an agent may open only its own place of {}", view.id),
                        Some(own_link),
                    ))
                }
            }
        }
        AgentRule::Yes => {}
    }
    let mut fill: Vec<&'static str> = Vec::new();
    for (name, held) in own_values(link) {
        match held {
            Some(v) if v.to_string() != who.agent_id => {
                return Err(refusal(
                    "not_allowed",
                    format!("an agent may name only itself as {name}; another agent's is not opened for it"),
                    None,
                ))
            }
            Some(_) => {}
            None => {
                if link.place.is_some_and(|p| p.params.iter().any(|x| x.name == name)) {
                    fill.push(name);
                }
            }
        }
    }
    for name in fill {
        link.params.insert(name.to_string(), Value::Text(who.agent_id.clone()));
    }
    if who.container && view.sensitivity == Sensitivity::Path {
        return Err(refusal(
            "not_allowed",
            "a container agent opens its files with Pane open's view and path, which shows a copy of a file in its workspace",
            None,
        ));
    }
    Ok(())
}

/// `path` canonicalised once, as forward slashes, or why it can't be opened (G7). `folder` says which kind it must be.
pub(crate) fn canonical_path(path: &str, folder: bool) -> Result<String, Refusal> {
    let bad = |why: &str| refusal("invalid_link", why.to_string(), None);
    let real = std::fs::canonicalize(path).map_err(|_| bad("no such file or folder"))?;
    let meta = std::fs::metadata(&real).map_err(|_| bad("no such file or folder"))?;
    if folder && !meta.is_dir() {
        return Err(bad("that is a file: open it with the Editor"));
    }
    if !folder && meta.is_dir() {
        return Err(bad("that is a folder: open it with Files"));
    }
    let text = real.to_string_lossy().to_string();
    let text = match text.strip_prefix(r"\\?\") {
        Some(rest) if rest.starts_with(r"UNC\") => return Err(bad("a network path is not opened by a link")),
        Some(rest) => rest.to_string(),
        None => text,
    };
    let text = if cfg!(windows) { text.replace('\\', "/") } else { text };
    // The structural rules again, on what the open will use.
    if deeplinks::path_problem(&text).is_some() {
        return Err(bad("the path is not an absolute local path a link may open"));
    }
    Ok(text)
}

/// G7 for the path destinations: the path is replaced by its canonical form, which the open uses.
fn check_path(link: &mut ParsedLink) -> Result<(), Refusal> {
    if link.view.sensitivity != Sensitivity::Path {
        return Ok(());
    }
    let Some(Value::Text(path)) = link.values.get("path").cloned() else { return Ok(()) };
    let folder = link.view.id == "files";
    let real = canonical_path(&path, folder)?;
    link.values.insert("path".to_string(), Value::Text(real));
    Ok(())
}

/// Where the open lands (G3, D1): `beside` the caller, or a new pane in its tab. A window is refused; a tab or a pane
/// that is not the caller's own is not reached: the open lands in the caller's own tab.
pub(crate) fn resolve_target(target: Option<&str>, who: &CallerFacts, pane_tab: impl Fn(&str) -> Option<String>) -> Result<(String, Option<String>), Refusal> {
    let target = target.map(str::trim).filter(|t| !t.is_empty()).unwrap_or("beside");
    let own_tab = who.tab_id.clone();
    let note = |what: &str| Some(format!("{what} is not this agent's own tab, so it opened in the agent's own tab"));
    match target.split_once(':').unwrap_or((target, "")) {
        ("beside", "") => Ok(("beside".into(), None)),
        ("new", "") => Ok(("new".into(), None)),
        ("window", _) => Err(refusal("another_window", "an agent's open stays in its own window: naming a window is refused in this build", None)),
        ("tab", tab) if !tab.is_empty() => Ok(("new".into(), if Some(tab.to_string()) == own_tab { None } else { note("that tab") })),
        ("pane", block) if !block.is_empty() => {
            if pane_tab(block).is_some_and(|t| Some(t) == own_tab) {
                Ok((format!("pane:{block}"), None))
            } else {
                Ok(("beside".into(), note("that pane's tab")))
            }
        }
        _ => Err(refusal("invalid_link", "target is beside, new, tab:<id>, pane:<id> or window:<id>", None)),
    }
}

/// A sliding window per agent.
struct Limiter {
    seen: Mutex<HashMap<String, VecDeque<Instant>>>,
}

impl Limiter {
    /// Whether `who` may open now (and counts it if so); when not, how long until it may.
    fn allow(&self, who: &str, now: Instant) -> Result<(), Duration> {
        let mut seen = self.seen.lock().unwrap_or_else(|e| e.into_inner());
        let q = seen.entry(who.to_string()).or_default();
        while q.front().is_some_and(|t| now.duration_since(*t) >= RATE_WINDOW) {
            q.pop_front();
        }
        if q.len() >= RATE_LIMIT {
            let oldest = *q.front().expect("full");
            return Err(RATE_WINDOW.saturating_sub(now.duration_since(oldest)));
        }
        q.push_back(now);
        Ok(())
    }
}

fn limiter() -> &'static Limiter {
    static LIMITER: OnceLock<Limiter> = OnceLock::new();
    LIMITER.get_or_init(|| Limiter { seen: Mutex::new(HashMap::new()) })
}

/// The same limit for an agent's floating opens and its `resize`, `move` and `place`: they and deep-link opens share
/// the ten a minute, so a loop of any of them is stopped. The text says when the next is allowed.
pub(crate) fn check_place_rate(agent_id: &str) -> Result<(), String> {
    limiter().allow(agent_id, Instant::now()).map_err(|wait| {
        format!("RATE_LIMITED: an agent opens and places at most {RATE_LIMIT} panes a minute; the next one is allowed in {} s", wait.as_secs().max(1))
    })
}

fn check_rate(agent_id: &str) -> Result<(), Refusal> {
    limiter().allow(agent_id, Instant::now()).map_err(|wait| {
        refusal(
            "rate_limited",
            format!("an agent opens at most {RATE_LIMIT} places a minute; the next one is allowed in {} s", wait.as_secs().max(1)),
            None,
        )
    })
}

/// Everything srv checks, in order, on `req` from `who`: the parsed link srv lets through, its target, and a note.
pub(crate) fn guard(req: &OpenLinkRequest, who: &CallerFacts, pane_tab: impl Fn(&str) -> Option<String>) -> Result<(ParsedLink, String, Option<String>), Refusal> {
    let mut link = deeplinks::parse_link(req.link.trim())?;
    check_rules(&mut link, who)?;
    let (target, note) = resolve_target(req.target.as_deref(), who, pane_tab)?;
    check_path(&mut link)?;
    check_rate(&who.agent_id)?;
    link.canonical = deeplinks::format_link(link.view, link.place, &link.values, &link.params, link.anchor.as_deref());
    Ok((link, target, note))
}

/// G9: the open, recorded as the agent.
fn record(state: &AppState, who: &CallerFacts, request_id: &str, link: &str, outcome: &DeepLinkOutcome) {
    let ok = matches!(outcome.outcome.as_str(), "opened" | "reused" | "queued");
    let why = outcome.reason.as_deref().map(|r| format!(": {r}")).unwrap_or_default();
    state.reactive_handler.log_fleet_action_audit(
        Some(&who.agent_id),
        "",
        outcome.block_id.as_deref().or(who.block_id.as_deref()).unwrap_or(""),
        "pane.open_link",
        ok,
        (!ok).then_some(outcome.reason.as_deref().unwrap_or("refused")),
        request_id,
        Some(&format!("{} {link}{why}", outcome.outcome)),
    );
}

/// The caller's own pane, tab and window, from srv's own store.
fn facts_of(state: &AppState, uid: &str, container: bool) -> CallerFacts {
    let agent_id = state.mstore.agent_names_by_id(uid).ok().flatten().map(|r| r.slug).filter(|s| !s.is_empty()).unwrap_or_else(|| uid.to_string());
    let own = app_api::agent_block::find_agent_block_by_uid(&state.mstore, uid).ok().flatten();
    let block_id = own.as_ref().map(|o| o.block.oid.clone());
    let tab_id = own.and_then(|o| o.tab_id);
    let window_id = tab_id.as_deref().and_then(|t| window_of_tab(state, t));
    CallerFacts { agent_id, container, block_id, tab_id, window_id }
}

fn window_of_tab(state: &AppState, tab_id: &str) -> Option<String> {
    use crate::backend::obj::{Window, Workspace};
    let ws = state
        .mstore
        .get_all::<Workspace>()
        .ok()?
        .into_iter()
        .find(|w| w.tabids.iter().chain(w.pinnedtabids.iter()).any(|t| t == tab_id))?;
    state.mstore.get_all::<Window>().ok()?.into_iter().find(|w| w.workspaceid == ws.oid).map(|w| w.oid)
}

fn tab_of_block(state: &AppState, block_id: &str) -> Option<String> {
    use crate::backend::obj::Tab;
    state.mstore.get_all::<Tab>().ok()?.into_iter().find(|t| t.blockids.iter().any(|b| b == block_id)).map(|t| t.oid)
}

/// `POST /api/v1/pane/open_link`: an agent opens a deep link. Answers the outcome of R3 (200) for every open the
/// guard decided, refused ones included, each recorded; 403 for a caller that is not an agent known by its token.
pub(super) async fn handle_pane_open_link(
    State(state): State<AppState>,
    grant: Option<axum::Extension<crate::backend::container::credential::ContainerGrant>>,
    caller: Option<axum::Extension<super::caller::Caller>>,
    Json(req): Json<OpenLinkRequest>,
) -> impl IntoResponse {
    let Some(uid) = caller.as_ref().and_then(|c| c.0.uid()).map(str::to_string) else {
        return (StatusCode::FORBIDDEN, Json(json!({ "error": "a deep link is opened for an agent known by its token; a person clicks the link" }))).into_response();
    };
    let who = facts_of(&state, &uid, grant.is_some());
    let request_id = uuid::Uuid::new_v4().to_string();
    let decided = guard(&req, &who, |b| tab_of_block(&state, b));
    let (link, target, note) = match decided {
        Ok(x) => x,
        Err(r) => {
            let outcome = DeepLinkOutcome::refused(req.link.chars().take(120).collect::<String>().as_str(), r.code, &r.text, r.remedy);
            record(&state, &who, &request_id, &req.link.chars().take(200).collect::<String>(), &outcome);
            return (StatusCode::OK, Json(json!(outcome))).into_response();
        }
    };
    let label = label_of(&link);
    let event = crate::backend::mps_events::DeepLinkOpenEvent {
        request_id: request_id.clone(),
        link: link.canonical.clone(),
        agent_id: who.agent_id.clone(),
        caller_block_id: who.block_id.clone(),
        tab_id: who.tab_id.clone(),
        window_id: who.window_id.clone(),
        target,
        expires_at_ms: agentmux_common::time::now_ms() + deeplink_open::KEEP_FOR.as_millis() as i64,
    };
    let late = {
        let (state, who, rid, canonical) = (state.clone(), who.clone(), request_id.clone(), link.canonical.clone());
        Box::new(move |o: &DeepLinkOutcome| record(&state, &who, &rid, &canonical, o))
    };
    let mut outcome = match deeplink_open::carry_out(&state, event, deeplink_open::WAIT_FOR, late).await {
        Some(o) => o,
        None => DeepLinkOutcome::queued(&label, "app_starting", "the window has not carried it out yet; it opens when it can"),
    };
    record(&state, &who, &request_id, &link.canonical, &outcome);
    if let Some(n) = note {
        outcome.remedy = Some(outcome.remedy.map(|r| format!("{n}. {r}")).unwrap_or(n));
    }
    let mut body = json!(outcome);
    body["link"] = json!(link.canonical);
    (StatusCode::OK, Json(body)).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agent(id: &str) -> CallerFacts {
        CallerFacts { agent_id: id.into(), container: false, block_id: Some("b-own".into()), tab_id: Some("t-own".into()), window_id: Some("w1".into()) }
    }

    fn parse(link: &str) -> ParsedLink {
        deeplinks::parse_link(link).unwrap_or_else(|e| panic!("{link}: {}", e.as_str()))
    }

    fn rules(link: &str, who: &CallerFacts) -> Result<ParsedLink, Refusal> {
        let mut l = parse(link);
        check_rules(&mut l, who).map(|()| l)
    }

    #[test]
    fn a_public_place_opens_for_any_agent() {
        for link in ["agentmux://settings/recording", "agentmux://kanban/task/12", "agentmux://vault/api-keys", "agentmux://memory/global", "agentmux://help"] {
            assert!(rules(link, &agent("a1")).is_ok(), "{link}");
        }
    }

    #[test]
    fn another_agents_personal_memory_is_refused_and_ones_own_opens() {
        let e = rules("agentmux://memory/personal?agent=b2", &agent("a1")).unwrap_err();
        assert_eq!(e.code, "not_allowed");
        assert!(rules("agentmux://memory/personal?agent=a1", &agent("a1")).is_ok());
    }

    #[test]
    fn personal_memory_without_an_agent_is_the_callers_own() {
        let l = rules("agentmux://memory/personal", &agent("a1")).unwrap();
        assert_eq!(l.params.get("agent"), Some(&Value::Text("a1".into())));
    }

    #[test]
    fn the_audit_opens_only_at_the_callers_own_rows() {
        assert!(rules("agentmux://audit/agent/a1", &agent("a1")).is_ok());
        assert_eq!(rules("agentmux://audit/agent/b2", &agent("a1")).unwrap_err().code, "not_allowed");
        for link in ["agentmux://audit", "agentmux://audit/message/m_1", "agentmux://audit/sound/s_1"] {
            let e = rules(link, &agent("a1")).unwrap_err();
            assert_eq!(e.code, "not_allowed", "{link}");
            assert_eq!(e.remedy.as_deref(), Some("agentmux://audit/agent/a1"), "{link}: the refusal says what would work");
        }
    }

    #[test]
    fn a_container_agent_gets_no_path_destination() {
        let mut who = agent("c1");
        who.container = true;
        for link in ["agentmux://editor/%2Fhome%2Fa.md", "agentmux://files/%2Fhome", "agentmux://media/%2Fhome%2Fa.png"] {
            assert_eq!(rules(link, &who).unwrap_err().code, "not_allowed", "{link}");
        }
        assert!(rules("agentmux://settings", &who).is_ok(), "a public view opens for a container agent too");
    }

    #[test]
    fn a_window_target_is_refused_and_another_tab_lands_in_the_callers_own() {
        let who = agent("a1");
        let no_panes = |_: &str| None;
        assert_eq!(resolve_target(Some("window:w2"), &who, no_panes).unwrap_err().code, "another_window");
        assert_eq!(resolve_target(None, &who, no_panes).unwrap(), ("beside".into(), None));
        assert_eq!(resolve_target(Some("new"), &who, no_panes).unwrap(), ("new".into(), None));
        let (t, note) = resolve_target(Some("tab:t-persons"), &who, no_panes).unwrap();
        assert_eq!(t, "new");
        assert!(note.unwrap().contains("own tab"));
        assert_eq!(resolve_target(Some("tab:t-own"), &who, no_panes).unwrap(), ("new".into(), None));
        let tabs = |b: &str| Some(if b == "b-mine" { "t-own".to_string() } else { "t-persons".to_string() });
        assert_eq!(resolve_target(Some("pane:b-mine"), &who, tabs).unwrap(), ("pane:b-mine".into(), None));
        let (t, note) = resolve_target(Some("pane:b-persons"), &who, tabs).unwrap();
        assert_eq!(t, "beside", "a pane in the person's tab is never the target");
        assert!(note.is_some());
        assert_eq!(resolve_target(Some("over-there"), &who, no_panes).unwrap_err().code, "invalid_link");
    }

    #[test]
    fn ten_opens_a_minute_and_the_eleventh_is_refused_until_the_window_moves() {
        let l = Limiter { seen: Mutex::new(HashMap::new()) };
        let t0 = Instant::now();
        for i in 0..RATE_LIMIT {
            assert!(l.allow("a1", t0 + Duration::from_millis(i as u64)).is_ok());
        }
        let wait = l.allow("a1", t0 + Duration::from_secs(1)).unwrap_err();
        assert!(wait <= RATE_WINDOW && wait > Duration::from_secs(50), "{wait:?}");
        assert!(l.allow("b2", t0 + Duration::from_secs(1)).is_ok(), "each agent has its own");
        assert!(l.allow("a1", t0 + RATE_WINDOW + Duration::from_millis(1)).is_ok(), "the window moved");
    }

    #[test]
    fn a_link_the_parser_refuses_is_refused_with_its_reason() {
        let who = agent("a1");
        let req = |link: &str| OpenLinkRequest { link: link.into(), target: None };
        let e = guard(&req("agentmux://settings?pane=b1"), &who, |_| None).unwrap_err();
        assert_eq!(e.code, "invalid_link");
        assert_eq!(e.text, deeplinks::REGISTRY.reason("unknown_param"));
        assert_eq!(guard(&req("agentmux://agent/a1"), &who, |_| None).unwrap_err().code, "not_yet");
    }

    #[test]
    fn a_path_is_opened_by_its_canonical_form_and_its_kind_is_checked() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.md");
        std::fs::write(&file, "x").unwrap();
        let sub = dir.path().join("sub");
        std::fs::create_dir(&sub).unwrap();
        // Written through `..`: the parser refuses that form in a link, so this is the guard's own check of what
        // the open will use, the canonical path.
        let real = canonical_path(&sub.join("..").join("a.md").to_string_lossy(), false).unwrap();
        assert!(real.ends_with("/a.md") || real.ends_with("\\a.md"), "{real}");
        assert!(!real.contains(".."), "{real}");
        assert!(!real.starts_with(r"\\?\"), "{real}");
        assert_eq!(canonical_path(&file.to_string_lossy(), true).unwrap_err().text, "that is a file: open it with the Editor");
        assert_eq!(canonical_path(&sub.to_string_lossy(), false).unwrap_err().text, "that is a folder: open it with Files");
        assert_eq!(canonical_path(&dir.path().join("gone.md").to_string_lossy(), false).unwrap_err().text, "no such file or folder");
    }

    #[cfg(unix)]
    #[test]
    fn a_symbolic_link_opens_where_it_leads_not_where_it_is_written() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("real.md");
        std::fs::write(&target, "x").unwrap();
        let alias = dir.path().join("alias.md");
        std::os::unix::fs::symlink(&target, &alias).unwrap();
        let real = canonical_path(&alias.to_string_lossy(), false).unwrap();
        assert!(real.ends_with("/real.md"), "{real}");
    }

    #[test]
    fn the_label_names_the_view_and_the_place_by_id() {
        assert_eq!(label_of(&parse("agentmux://settings/recording")), "settings › recording");
        assert_eq!(label_of(&parse("agentmux://kanban/task/7")), "kanban › task");
        assert_eq!(label_of(&parse("agentmux://help")), "help");
    }
}

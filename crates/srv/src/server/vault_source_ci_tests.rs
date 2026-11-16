// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Who may start the read of a CI secret, and its line in Audit
//! (docs/specs/SPEC_VAULT_LINKED_ENTRIES_FLEET_VAULT_AND_BATCH_IMPORT_2026_11_15.md §2.6). Every repository and
//! secret here is invented.

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use super::*;
use crate::backend::reactive::handler::get_global_handler;
use crate::backend::reactive::AuditLogEntry;
use crate::backend::vault::source_link::{GITHUB_SECRET, KEY, KIND, LOCATION, PRESENT};
use crate::backend::vault::{create, linked, seal, NewEntry};
use crate::identity::aws_broker::mint::{RunError, RunOutput, RunSpec};

const VALUE: &str = "ci-check-sentinel-5531";
const REASON: &str = "see whether the store key changed before task 31007";

/// A stand-in GitHub CLI that knows one secret, and counts its runs.
#[derive(Default)]
struct Gh {
    runs: AtomicUsize,
}

impl Runner for Gh {
    fn which(&self, program: &str) -> Option<PathBuf> {
        (program == "gh").then(|| PathBuf::from("/bin/gh"))
    }

    async fn run(&self, _spec: RunSpec) -> Result<RunOutput, RunError> {
        self.runs.fetch_add(1, Ordering::SeqCst);
        let printed = "HTTP/2.0 200 OK\r\n\r\n{\"name\": \"STORE_KEY\", \"updated_at\": \"2026-10-01T09:00:00Z\"}";
        Ok(RunOutput { success: true, stdout: printed.as_bytes().to_vec(), stderr: String::new() })
    }
}

struct Setup {
    state: AppState,
    /// An entry that holds a value and names a CI secret, and a link to the same secret.
    entry: String,
    link: String,
    label: String,
}

fn setup() -> Setup {
    seal::use_test_key();
    let state = crate::server::tests::test_state();
    let label = format!("Store upload key {}", uuid::Uuid::new_v4().simple());
    let place = [(KIND, GITHUB_SECRET), (LOCATION, "example-org/example-app"), (KEY, "STORE_KEY")];
    let fields = |more: &[(&str, &str)]| place.iter().chain(more).map(|(k, v)| (k.to_string(), v.to_string())).collect();
    let made = |label: String, fields: std::collections::BTreeMap<String, String>, value: Option<&str>| {
        let parts = value.map(|v| seal::Parts([("value".to_string(), v.to_string())].into())).unwrap_or_default();
        create(&state.identity_store, NewEntry { kind: "api-key".into(), label, fields, tags: vec![], parts }).unwrap()
    };
    let entry = made(label.clone(), fields(&[]), Some(VALUE));
    let link = made(format!("{label} (link)"), fields(&[(linked::LINK, linked::ONLY)]), None);
    Setup { state, entry, link, label }
}

fn who(container: bool) -> Who {
    let agent = format!("checker-{}", uuid::Uuid::new_v4().simple());
    Who { agent: agent.clone(), pane: String::new(), slug: agent, uid: String::new(), container }
}

fn plan(s: &Setup, id: &str, who: &Who) -> CheckPlan {
    source_link::plan_for_agent(&s.state.identity_store, id, &who.agent, Some(REASON)).unwrap()
}

/// The Audit's action lines of CI secret reads by `actor` that name `label`.
fn audited(actor: &Actor, label: &str) -> Vec<AuditLogEntry> {
    let named = |e: &AuditLogEntry| e.outcome.as_deref() == Some(AUDIT_ACTION) && e.reason.as_deref().is_some_and(|r| r.contains(label));
    let by = actor.label();
    get_global_handler().get_audit_log(usize::MAX).into_iter().filter(|e| e.event_kind == "action" && e.source_agent.as_deref() == Some(by.as_str()) && named(e)).collect()
}

fn agent(who: &Who) -> Actor {
    Actor::Agent(who.slug.clone())
}

#[tokio::test]
async fn a_container_agent_reads_a_ci_secret_only_with_the_persons_standing_approval() {
    let s = setup();
    let (boxed, gh) = (who(true), Gh::default());

    // No approval: nothing is run, nothing is recorded, and the answer says why and what to do.
    let refused = run(&s.state, &gh, plan(&s, &s.entry, &boxed), &boxed).await;
    assert_eq!(refused.outcome, NOT_STARTED);
    assert!(refused.why.contains("the person's GitHub login") && refused.why.contains("Ask for the entry through the Vault"), "{}", refused.why);
    assert!(refused.why.ends_with("It has not been checked yet."), "{}", refused.why);
    assert_eq!((gh.runs.load(Ordering::SeqCst), audited(&agent(&boxed), &s.label).len()), (0, 0));
    // A link holds no value, so it can have no such approval: the person checks it.
    let link = run(&s.state, &gh, plan(&s, &s.link, &boxed), &boxed).await;
    assert_eq!(link.outcome, NOT_STARTED);
    assert!(link.why.contains("holds no value") && link.why.contains("ask the person to check it"), "{}", link.why);
    assert_eq!(gh.runs.load(Ordering::SeqCst), 0);

    // The person answered Always to this agent's use of the entry's value: the read runs, and is one Audit line.
    let created = s.state.identity_store.vault_get(&s.entry).unwrap().unwrap().created_at;
    let key = super::super::shell_secrets::grant_key(&boxed.agent, "value");
    s.state.identity_store.vault_grant_add(&s.entry, &key, agentmux_common::time::now_ms(), created).unwrap();
    let read = run(&s.state, &gh, plan(&s, &s.entry, &boxed), &boxed).await;
    assert_eq!((read.outcome, read.last_changed.as_deref(), gh.runs.load(Ordering::SeqCst)), (PRESENT, Some("2026-10-01"), 1));
    let rows = audited(&agent(&boxed), &s.label);
    assert_eq!(rows.len(), 1);
    let line = rows[0].reason.clone().unwrap();
    assert!(line.contains("STORE_KEY of example-org/example-app") && line.contains(&s.entry) && line.ends_with(": present"), "{line}");
    assert!(rows[0].success && !format!("{rows:?}").contains(VALUE));
    // The approval is one agent's: another container agent is still told to ask.
    let other = who(true);
    assert_eq!(run(&s.state, &gh, plan(&s, &s.entry, &other), &other).await.outcome, NOT_STARTED);
}

#[tokio::test]
async fn a_native_agents_and_the_persons_read_are_each_one_audit_line() {
    let s = setup();
    let (native, gh) = (who(false), Gh::default());
    // A native agent already runs as the person: its check reads, a link's too.
    assert_eq!(run(&s.state, &gh, plan(&s, &s.entry, &native), &native).await.outcome, PRESENT);
    assert_eq!(run(&s.state, &gh, plan(&s, &s.link, &native), &native).await.outcome, PRESENT);
    assert_eq!((gh.runs.load(Ordering::SeqCst), audited(&agent(&native), &s.label).len()), (2, 2));

    let (state, link) = (s.state.clone(), s.link.clone());
    let by_person = tokio::task::spawn_blocking(move || person_check_with(&state, &Gh::default(), &link)).await.unwrap().unwrap();
    assert_eq!(by_person.outcome, PRESENT);
    let rows = audited(&Actor::Operator, &s.label);
    assert_eq!(rows.len(), 1);
    assert!(rows[0].reason.as_deref().unwrap().contains(&s.link));
}

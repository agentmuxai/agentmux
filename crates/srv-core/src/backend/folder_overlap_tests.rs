// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::*;

/// One folder for every test's choices: each test names its own agents, so none reads another's.
fn choices() {
    static DIR: std::sync::OnceLock<tempfile::TempDir> = std::sync::OnceLock::new();
    let dir = DIR.get_or_init(|| tempfile::tempdir().unwrap());
    set_choices_dir_for_test(Some(dir.path().to_path_buf()));
}

fn definition(kind: &str, name: &str, working_directory: &str, volumes: &str) -> AgentDefinition {
    let id = format!("{name}-{}", uuid::Uuid::new_v4().simple());
    AgentDefinition {
        seat: None,
        conversation_visibility: crate::backend::storage::agents::default_conversation_visibility(),
        id,
        slug: String::new(),
        name: name.to_string(),
        icon: String::new(),
        provider: "claude".to_string(),
        description: String::new(),
        working_directory: working_directory.to_string(),
        shell: String::new(),
        provider_flags: String::new(),
        auto_start: 0,
        restart_on_crash: 0,
        idle_timeout_minutes: 0,
        created_at: 1_700_000_000_000,
        agent_type: kind.to_string(),
        environment: String::new(),
        agent_bus_id: String::new(),
        is_seeded: 0,
        accounts: String::new(),
        parent_id: String::new(),
        branch_label: String::new(),
        updated_at: 1_700_000_000_000,
        user_hidden: 0,
        container_image: String::new(),
        container_volumes: volumes.to_string(),
        container_name: String::new(),
        use_ambient_login: 0,
        model_vendor_base_url: String::new(),
        memory_id: String::new(),
    }
}

fn saved(store: &Store, mut def: AgentDefinition) -> AgentDefinition {
    store.agent_def_insert(&mut def).unwrap();
    def
}

fn path(dir: &tempfile::TempDir, rest: &str) -> String {
    dir.path().join(rest).to_string_lossy().to_string()
}

#[test]
fn a_host_agent_in_a_container_agents_workspace_or_in_a_folder_that_holds_it_overlaps_it() {
    choices();
    let store = Store::open_in_memory().unwrap();
    let root = tempfile::tempdir().unwrap();
    let container = saved(&store, definition("container", "Aria", &path(&root, "shared/aria"), "[]"));
    let inside = saved(&store, definition("host", "Hosty", &path(&root, "shared/aria/sub"), "[]"));
    let holder = saved(&store, definition("host", "Holdy", &path(&root, "shared"), "[]"));
    let apart = saved(&store, definition("host", "Aparty", &path(&root, "elsewhere"), "[]"));

    for host in [&inside, &holder] {
        let found = of(&store, host, None);
        assert_eq!(found.len(), 1, "{}", host.name);
        assert_eq!((found[0].host_id.as_str(), found[0].container_id.as_str()), (host.id.as_str(), container.id.as_str()));
        assert_eq!((found[0].host_name.as_str(), found[0].container_name.as_str()), (host.name.as_str(), "Aria"));
    }
    assert!(of(&store, &apart, None).is_empty(), "a folder of its own");
}

#[test]
fn a_writable_volume_is_a_folder_the_container_agent_can_write_and_a_read_only_one_is_not() {
    choices();
    let store = Store::open_in_memory().unwrap();
    let root = tempfile::tempdir().unwrap();
    let (rw, ro) = (path(&root, "rw"), path(&root, "ro"));
    let volumes = serde_json::json!([format!("{rw}:/data"), format!("{ro}:/docs:ro")]).to_string();
    saved(&store, definition("container", "Vola", &path(&root, "vola-workspace"), &volumes));
    let writes = saved(&store, definition("host", "Writes", &rw, "[]"));
    let reads = saved(&store, definition("host", "Reads", &ro, "[]"));
    assert_eq!(of(&store, &writes, None).len(), 1);
    assert!(of(&store, &reads, None).is_empty(), "a read-only volume can't be changed by the container agent");
}

#[test]
fn saving_a_container_agents_folders_finds_the_host_agents_that_share_them() {
    choices();
    let store = Store::open_in_memory().unwrap();
    let root = tempfile::tempdir().unwrap();
    let host = saved(&store, definition("host", "Hosta", &path(&root, "work"), "[]"));
    saved(&store, definition("host", "Hostb", &path(&root, "other"), "[]"));
    let moved = definition("container", "Cona", &path(&root, "work/in"), "[]");
    let found = of(&store, &moved, None);
    assert_eq!(found.iter().map(|o| o.host_id.as_str()).collect::<Vec<_>>(), vec![host.id.as_str()]);
    assert_eq!(found[0].container_id, moved.id);
}

#[test]
fn a_host_agent_that_opens_in_its_own_folder_is_asked_of_that_folder_not_of_its_blank_one() {
    choices();
    let store = Store::open_in_memory().unwrap();
    let root = tempfile::tempdir().unwrap();
    saved(&store, definition("container", "Cb", &path(&root, "cb"), "[]"));
    let blank = saved(&store, definition("host", "Blanky", "", "[]"));
    assert!(of(&store, &blank, None).is_empty(), "a blank folder names none");
    assert_eq!(of(&store, &blank, Some(&root.path().join("cb"))).len(), 1);
}

#[test]
fn the_persons_choice_is_kept_for_that_pair_of_agents_only() {
    choices();
    let store = Store::open_in_memory().unwrap();
    let root = tempfile::tempdir().unwrap();
    let one = saved(&store, definition("container", "One", &path(&root, "w"), "[]"));
    let two = saved(&store, definition("container", "Two", &path(&root, "w"), "[]"));
    let host = saved(&store, definition("host", "Hostc", &path(&root, "w"), "[]"));
    assert_eq!(unchosen(of(&store, &host, None)).len(), 2);

    accept(&host.id, &one.id).unwrap();
    let still = unchosen(of(&store, &host, None));
    assert_eq!(still.iter().map(|o| o.container_id.as_str()).collect::<Vec<_>>(), vec![two.id.as_str()], "the other pair still asks");
    assert!(accepted(&host.id, &one.id) && !accepted(&one.id, &host.id), "a pair is ordered: host then container");

    forget_agent(&one.id);
    assert_eq!(unchosen(of(&store, &host, None)).len(), 2, "the container agent was deleted: its choices go with it");
    accept(&host.id, &two.id).unwrap();
    forget_agent(&one.id);
    assert!(accepted(&host.id, &two.id), "another agent's choices stay");
}

#[test]
fn a_caller_that_is_not_the_person_is_told_both_agents_what_it_means_and_what_to_do_instead() {
    choices();
    let store = Store::open_in_memory().unwrap();
    let root = tempfile::tempdir().unwrap();
    let container = saved(&store, definition("container", "Cont", &path(&root, "w"), "[]"));
    let host = saved(&store, definition("host", "Hostd", &path(&root, "w/x"), "[]"));
    let why = check(&store, &host, None).unwrap_err();
    assert!(why.contains("Hostd (host)") && why.contains("Cont (container)"), "{why}");
    assert!(why.contains("can change what the host agent runs"), "{why}");
    assert!(why.contains("separate workspaces") && why.contains(":ro"), "{why}");
    accept(&host.id, &container.id).unwrap();
    assert!(check(&store, &host, None).is_ok(), "once the person has gone ahead");
}

#[test]
fn two_container_agents_or_two_host_agents_sharing_a_folder_are_not_asked_about() {
    choices();
    let store = Store::open_in_memory().unwrap();
    let root = tempfile::tempdir().unwrap();
    let c1 = saved(&store, definition("container", "Ce1", &path(&root, "w"), "[]"));
    saved(&store, definition("container", "Ce2", &path(&root, "w"), "[]"));
    let h1 = saved(&store, definition("host", "He1", &path(&root, "h"), "[]"));
    saved(&store, definition("host", "He2", &path(&root, "h"), "[]"));
    assert!(of(&store, &c1, None).is_empty() && of(&store, &h1, None).is_empty());
}

#[test]
fn a_container_worker_as_spawn_records_it_is_a_container_agent_here() {
    choices();
    let store = Store::open_in_memory().unwrap();
    let root = tempfile::tempdir().unwrap();
    // What `agent_spawn::container::apply` leaves on a worker's definition: the type, the environment and the image.
    let mut worker = definition("container", "Pell", &path(&root, "pell"), &serde_json::json!([format!("{}:/shared", path(&root, "shared"))]).to_string());
    worker.environment = "docker".to_string();
    worker.container_image = "agentmux/developer:1".to_string();
    let worker = saved(&store, worker);
    let host = saved(&store, definition("host", "Leady", &path(&root, "shared/leader"), "[]"));
    let found = of(&store, &host, None);
    assert_eq!((found.len(), found[0].container_id.as_str(), found[0].container_name.as_str()), (1, worker.id.as_str(), "Pell"));
    assert_eq!(of(&store, &worker, None).len(), 1, "and the worker's own save finds the host agent");
}

// A choice file is not proof the person made it: the audit log has to record the go-ahead.

use crate::backend::reactive::AuditLogEntry;
use std::io::Write;

fn go_ahead_row(host_id: &str, container_id: &str, by: Option<&str>) -> AuditLogEntry {
    AuditLogEntry {
        source_agent: by.map(str::to_string),
        outcome: Some(GO_AHEAD_ACTION.to_string()),
        event_kind: "action".to_string(),
        reason: Some(row_note(GO_AHEAD_ACTION, (host_id, "Hosty"), (container_id, "Aria"))),
        ..Default::default()
    }
}

fn recorded_by_the_window(host_id: &str, container_id: &str) -> Vec<AuditLogEntry> {
    let prefix = row_prefix(host_id, container_id);
    crate::backend::reactive::get_global_handler()
        .get_audit_log(usize::MAX)
        .into_iter()
        .filter(|e| e.outcome.as_deref() == Some(GO_AHEAD_ACTION) && e.reason.as_deref().is_some_and(|r| r.starts_with(&prefix)))
        .collect()
}

#[test]
fn a_go_ahead_is_recorded_as_the_windows_and_the_choice_counts() {
    choices();
    let host = format!("h-{}", uuid::Uuid::new_v4().simple());
    let container = format!("c-{}", uuid::Uuid::new_v4().simple());
    accept_as((&host, "Hosty"), (&container, "Aria")).unwrap();
    let rows = recorded_by_the_window(&host, &container);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].source_agent, None, "the person's window, not an agent");
    assert_eq!(rows[0].event_kind, "action");
    let note = rows[0].reason.as_deref().unwrap();
    assert!(note.contains("Hosty") && note.contains("Aria") && note.contains(&host) && note.contains(&container), "both agents by id and name: {note}");
    assert!(accepted(&host, &container));
    assert!(choice_file(&host, &container).unwrap().is_file(), "and the choice is kept");
}

#[test]
fn a_choice_file_with_no_record_does_not_count_and_the_pair_asks_again_and_says_why() {
    choices();
    set_audit_files_for_test(Vec::new());
    let store = Store::open_in_memory().unwrap();
    let root = tempfile::tempdir().unwrap();
    let container = saved(&store, definition("container", "Aria", &path(&root, "w"), "[]"));
    let host = saved(&store, definition("host", "Hosty", &path(&root, "w"), "[]"));
    write_choice_without_a_record_for_test(&host.id, &container.id);

    assert!(!accepted(&host.id, &container.id), "a file an agent can write is not the person's word");
    assert!(untrusted(&host.id, &container.id));
    let asking = unchosen(of(&store, &host, None));
    assert_eq!(asking.len(), 1, "the pair asks again");
    assert!(asking[0].untrusted);
    let told = refusal(&asking);
    assert!(told.contains("Hosty and Aria") && told.contains("no record that the person made it"), "{told}");
    assert!(check(&store, &host, None).is_err());

    // The person goes ahead for real: the same pair then counts and says nothing of it.
    accept_as((&host.id, "Hosty"), (&container.id, "Aria")).unwrap();
    assert!(accepted(&host.id, &container.id) && !untrusted(&host.id, &container.id));
    assert!(check(&store, &host, None).is_ok());
}

#[test]
fn a_record_in_the_audit_file_counts_after_a_restart_and_only_the_windows_for_that_pair_does() {
    choices();
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("jekt-audit.jsonl");
    let (host, container) = ("h-restart", "c-restart");
    let lines = [
        serde_json::to_string(&go_ahead_row(host, "c-another", None)).unwrap(),
        serde_json::to_string(&go_ahead_row("h-another", container, None)).unwrap(),
        "{torn".to_string(),
    ];
    std::fs::write(&file, lines.join("\n") + "\n").unwrap();
    set_audit_files_for_test(vec![file.clone()]);
    write_choice_without_a_record_for_test(host, container);
    forget_what_was_recorded_for_test();
    assert!(!accepted(host, container), "a row for another pair is not a row for this one");

    let by_an_agent = serde_json::to_string(&go_ahead_row(host, container, Some("agent:korp"))).unwrap();
    std::fs::OpenOptions::new().append(true).open(&file).unwrap().write_all(format!("{by_an_agent}\n").as_bytes()).unwrap();
    assert!(!accepted(host, container), "a row an agent made is not the window's");

    let by_the_window = serde_json::to_string(&go_ahead_row(host, container, None)).unwrap();
    std::fs::OpenOptions::new().append(true).open(&file).unwrap().write_all(format!("{by_the_window}\n").as_bytes()).unwrap();
    forget_what_was_recorded_for_test();
    assert!(accepted(host, container), "the row survived the restart, in the file");

    forget_agent(container);
    assert!(!accepted(host, container), "deleting the agent forgets the choice, and what was found on record");
    set_audit_files_for_test(Vec::new());
}

#[test]
fn a_host_agents_start_waits_for_the_person_and_says_so_until_the_person_has_gone_ahead() {
    choices();
    let store = Store::open_in_memory().unwrap();
    let root = tempfile::tempdir().unwrap();
    let container = saved(&store, definition("container", "Aria", &path(&root, "w"), "[]"));
    let host = saved(&store, definition("host", "Hosty", &path(&root, "w/work"), "[]"));
    let apart = saved(&store, definition("host", "Apart", &path(&root, "elsewhere"), "[]"));

    let why = waiting_reason(&store, &host).expect("it shares a folder");
    assert!(why.starts_with(OPEN_REFUSAL_CODE) && why.contains("waits for the person's go-ahead for a shared folder"), "{why}");
    assert!(why.contains("Hosty (host) and Aria (container)"), "{why}");
    assert_eq!(waiting_reason(&store, &apart), None, "a host agent that shares nothing waits for nothing");
    assert_eq!(waiting_reason(&store, &container), None, "a container agent is asked when its folders are saved");

    accept_as((&host.id, "Hosty"), (&container.id, "Aria")).unwrap();
    assert_eq!(waiting_reason(&store, &host), None, "the person went ahead");
}

use super::*;
use crate::backend::memory::delivery::{compose_items_tail, split_into_parts_counted, Layout, Reason, StartupCarry, HOOK_PARTS, MAX_PART_CHARS};

const CAP: usize = HOOK_PARTS * MAX_PART_CHARS;

fn personal(name: &str, text: &str) -> Entry {
    Entry { label: name.into(), tier: Tier::Personal, text: text.into(), name: name.into(), system: false, bundle_id: None, path: Some(format!("/mem/{name}")) }
}

fn global_named(name: &str, text: &str) -> Entry {
    Entry { label: format!("[Workspace] {name}"), tier: Tier::Global, text: text.into(), name: name.into(), system: false, bundle_id: Some(format!("id-{name}")), path: None }
}

fn global(text: &str) -> Entry {
    global_named("g", text)
}

fn with_front(kind: &str, description: &str, body_chars: usize) -> String {
    format!("---\nname: n\ndescription: {description}\nmetadata:\n  type: {kind}\n---\n{}", "x".repeat(body_chars))
}

fn always_front(description: &str, body_chars: usize) -> String {
    format!("---\nname: n\ndescription: {description}\nmetadata:\n  type: feedback\n  deliver: always\n---\n{}", "x".repeat(body_chars))
}

fn composed_text(fitted: &Fitted, carry: StartupCarry, summary: Option<&str>) -> String {
    composed_text_opt(fitted, carry, summary).unwrap_or_default()
}

fn composed_text_opt(fitted: &Fitted, carry: StartupCarry, summary: Option<&str>) -> Option<String> {
    let layout = Layout {
        carry,
        index: IndexPlan { files: &fitted.indexed, over_limit: &fitted.over_limit, always: fitted.always, tool_only: false },
        tail: fitted.tail.as_deref(),
    };
    compose_items_tail(&fitted.kept, Reason::Compact, summary, &layout).map(|c| c.text)
}

#[test]
fn frontmatter_is_read_at_any_indent_and_only_from_the_top_block() {
    let t = "---\nname: a\ndescription: \"A thing\"\nmetadata:\n  type: project\n---\ndescription: not this\n";
    assert_eq!(frontmatter_value(t, "description").as_deref(), Some("A thing"));
    assert_eq!(frontmatter_value(t, "type").as_deref(), Some("project"));
    assert_eq!(frontmatter_value(t, "missing"), None);
    assert_eq!(frontmatter_value("no front\ndescription: x\n", "description"), None, "no block, no value");
    assert_eq!(frontmatter_value("---\nname: a\n---\ndescription: body\n", "description"), None, "the body is not read");
}

/// The 11-17 incident: Global Memory filled the delivery and every Personal file came out as "not sent".
#[test]
fn personal_memory_is_an_index_and_a_big_file_no_longer_crowds_anything_out() {
    let entries = vec![
        global(&"G".repeat(20_000)),
        personal("front-door-state-pm.md", &with_front("project", "READ FIRST", 3_000)),
        personal("leader-delegates-code.md", &with_front("feedback", "Leaders delegate code", 1_000)),
        personal("front-door-state-old.md", &with_front("project", "Old snapshot", 50_000)),
    ];
    let fitted = fit(entries, CAP, 0, false, &|_| None);
    assert!(fitted.skipped.is_empty());
    assert_eq!(fitted.kept.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(), ["g"], "only Global rides in full");
    assert_eq!(fitted.indexed.len(), 3, "every Personal file is in the index");
    let text = composed_text(&fitted, StartupCarry::None, None);
    for name in ["front-door-state-pm.md", "leader-delegates-code.md", "front-door-state-old.md"] {
        assert_eq!(text.matches(&format!("- {name} ")).count(), 1, "{name} is listed once");
    }
    assert!(text.contains("READ FIRST") && !text.contains(&"x".repeat(100)), "the descriptions, not the bodies");
}

#[test]
fn files_marked_always_go_in_full_within_their_limit_and_the_rest_are_named() {
    let mut entries: Vec<Entry> = (0..7).map(|i| personal(&format!("rule{i}.md"), &always_front(&format!("Rule {i}"), 500))).collect();
    entries.push(personal("plain.md", &with_front("project", "A plain file", 500)));
    let fitted = fit(entries, CAP, 0, false, &|_| None);
    assert_eq!(fitted.always, ALWAYS_MAX_FILES);
    assert_eq!(fitted.kept.len(), ALWAYS_MAX_FILES);
    assert_eq!(fitted.over_limit.len(), 2, "{:?}", fitted.over_limit);
    assert_eq!(fitted.indexed.len(), 3, "the plain file and the two over the limit");
    let text = composed_text(&fitted, StartupCarry::None, None);
    for name in &fitted.over_limit {
        assert!(text.contains(&format!("- {name} ")) && text.contains("marked always, but over the limit"), "{name} is in the index with the reason");
    }
}

#[test]
fn marked_files_over_the_character_limit_are_not_dropped() {
    let entries: Vec<Entry> = (0..3).map(|i| personal(&format!("big{i}.md"), &always_front("big", 3_500))).collect();
    let fitted = fit(entries, CAP, 0, false, &|_| None);
    assert_eq!(fitted.always, 1, "one fits in 6,000 characters");
    assert_eq!(fitted.over_limit.len(), 2);
    assert_eq!(fitted.always + fitted.indexed.len(), 3, "none vanishes");
}

#[test]
fn global_memory_that_does_not_fit_is_left_out_whole_and_named_with_how_to_read_it() {
    // 10 entries of 9 KB (90 KB) cannot all ride in 72 KB.
    let entries: Vec<Entry> = (0..10).map(|i| global_named(&format!("entry-{i}"), &"G".repeat(9_000))).collect();
    let fitted = fit(entries, CAP, 0, false, &|_| None);
    assert!(!fitted.skipped.is_empty() && fitted.kept.len() + fitted.skipped.len() == 10);
    assert!(fitted.kept.iter().all(|e| e.text.len() == 9_000), "an entry is whole or not sent: none is cut");
    let tail = fitted.tail.clone().unwrap();
    for e in &fitted.skipped {
        assert!(tail.contains(&format!("- {} (id-{})", e.name, e.name)), "{tail}");
    }
    assert!(tail.contains("scope global"), "{tail}");

    // The whole composed delivery, the note included, goes out uncut.
    let text = composed_text(&fitted, StartupCarry::None, None);
    let (parts, delivered) = split_into_parts_counted(&text, MAX_PART_CHARS, HOOK_PARTS);
    assert_eq!(delivered, text.chars().count(), "nothing is cut after the fit");
    assert!(parts.iter().all(|p| !p.contains("memory cut here")));
}

#[test]
fn nothing_is_dropped_whatever_the_sizes() {
    for (globals, global_kb, files) in [(0usize, 0usize, 0usize), (37, 2, 11), (20, 6, 60), (50, 2, 200), (5, 30, 9)] {
        let mut entries: Vec<Entry> = (0..globals).map(|i| global_named(&format!("g{i}"), &"G".repeat(global_kb * 1_000))).collect();
        entries.extend((0..files).map(|i| personal(&format!("f{i:03}.md"), &with_front("project", &format!("File {i}"), 800 * (i % 5)))));
        let fitted = fit(entries, CAP, 0, true, &|_| Some(1));
        let text = composed_text(&fitted, StartupCarry::None, None);
        let (parts, delivered) = split_into_parts_counted(&text, MAX_PART_CHARS, HOOK_PARTS);
        for i in 0..files {
            assert_eq!(text.matches(&format!("- f{i:03}.md ")).count(), 1, "file {i} in the index once ({globals}, {global_kb}, {files})");
        }
        for e in &fitted.skipped {
            assert!(text.contains(&format!("- {} (", e.name)), "{} is named ({globals}, {global_kb}, {files})", e.name);
        }
        assert_eq!(fitted.kept.len() + fitted.skipped.len(), globals, "every Global entry is sent or named");
        if fitted.skipped.is_empty() {
            assert_eq!(delivered, text.chars().count(), "uncut when nothing was left out ({globals}, {global_kb}, {files})");
            assert!(parts.iter().all(|p| !p.contains("memory cut here")));
        }
    }
}

#[test]
fn the_running_summary_is_kept_room_for() {
    let entries: Vec<Entry> = (0..12).map(|i| global_named(&format!("g{i}"), &"G".repeat(7_000))).collect();
    let without = fit(entries.clone(), CAP, 0, false, &|_| None).kept.len();
    let with = fit(entries, CAP, 20_000, false, &|_| None).kept.len();
    assert!(with < without, "{with} < {without}");
}

fn seen(name: &str, chars: usize, kind: &str, age: Option<i64>) -> Seen {
    Seen { name: name.into(), chars, kind: Some(kind.into()), age_days: age }
}

#[test]
fn a_small_fresh_memory_gets_no_nudge() {
    assert_eq!(hygiene_note(&[seen("a.md", 2_000, "feedback", Some(1)), seen("b.md", 3_000, "project", Some(2))]), None);
    // An old feedback or reference file is not stale: only project snapshots age out.
    assert_eq!(hygiene_note(&[seen("rule.md", 500, "feedback", Some(300))]), None);
}

#[test]
fn the_nudge_names_what_is_large_or_stale_and_says_nothing_is_deleted() {
    let note = hygiene_note(&[seen("front-door.md", 50_000, "project", Some(4)), seen("old-plan.md", 1_000, "project", Some(12)), seen("rule.md", 500, "feedback", Some(40))]).unwrap();
    assert!(note.starts_with("[AgentMux] Your Personal Memory is 51 KB (3 files"), "{note}");
    assert!(note.contains("front-door.md (49 KB)"), "{note}");
    assert!(note.contains("old-plan.md (12 days)"), "{note}");
    assert!(!note.contains("rule.md"), "{note}");
    assert!(note.contains("never deletes memory"), "{note}");
    // Over the total budget alone is enough.
    let many: Vec<Seen> = (0..6).map(|i| seen(&format!("f{i}.md"), 7_500, "feedback", Some(1))).collect();
    assert!(hygiene_note(&many).unwrap().contains("Your Personal Memory is 44 KB (6 files"));
}

#[test]
fn the_nudge_rides_with_the_delivery_only_on_a_compaction() {
    let entries = vec![personal("big.md", &with_front("project", "d", 20_000))];
    let compact = fit(entries.clone(), CAP, 0, true, &|_| Some(30));
    assert!(compact.tail.as_deref().unwrap().contains("Your Personal Memory is"));
    let startup = fit(entries, CAP, 0, false, &|_| Some(30));
    assert_eq!(startup.tail, None);
}

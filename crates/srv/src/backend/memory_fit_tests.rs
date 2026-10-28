use super::*;
use crate::backend::memory_delivery::{compose_items_tail, split_into_parts_counted, Reason};

fn personal(name: &str, text: &str) -> Entry {
    Entry { label: name.into(), tier: Tier::Personal, text: text.into(), name: name.into(), system: false, bundle_id: None, path: Some(format!("/mem/{name}")) }
}

fn global(text: &str) -> Entry {
    Entry { label: "[Workspace] g".into(), tier: Tier::Global, text: text.into(), name: "g".into(), system: false, bundle_id: None, path: None }
}

fn with_front(kind: &str, description: &str, body_chars: usize) -> String {
    format!("---\nname: n\ndescription: {description}\nmetadata:\n  type: {kind}\n---\n{}", "x".repeat(body_chars))
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

#[test]
fn a_personal_file_over_the_cap_is_cut_with_a_line_saying_how_much_is_left() {
    let big = personal("big.md", &format!("{}\n{}", "a".repeat(5_000), "b".repeat(45_000)));
    let capped = cap_entry(big);
    assert!(capped.text.chars().count() < FILE_CAP_CHARS + 200, "{}", capped.text.chars().count());
    assert!(capped.text.starts_with("aaaa"));
    assert!(capped.text.ends_with("read the file: /mem/big.md]"), "{}", &capped.text[capped.text.len() - 80..]);
    assert!(capped.text.contains("[cut: 4"), "about 42 KB are left: {}", &capped.text[capped.text.len() - 80..]);
    let small = personal("small.md", "short");
    assert_eq!(cap_entry(small.clone()), small, "under the cap: untouched");
    let g = global(&"g".repeat(30_000));
    assert_eq!(cap_entry(g.clone()), g, "Global Memory is the startup file's text, never cut");
}

#[test]
fn the_cap_cuts_at_a_line_end_when_one_is_near() {
    let line = format!("{}\n", "l".repeat(99));
    let capped = cap_entry(personal("lines.md", &line.repeat(200)));
    let body = capped.text.split("\n[cut:").next().unwrap();
    assert!(body.ends_with('l') && !body.ends_with('\n'), "cut after a whole line");
    assert_eq!(body.chars().count() % 100, 99, "a whole number of lines");
}

/// The 10-28 incident: a stale 50 KB snapshot used the whole budget and every later file was "not sent".
#[test]
fn one_big_stale_file_no_longer_crowds_out_the_rest() {
    // Newest first, as the caller leaves them: a "read first" file and rules, then the stale snapshot.
    let mut entries = vec![
        personal("front-door-state-pm.md", &with_front("project", "READ FIRST", 3_000)),
        personal("leader-delegates-code.md", &with_front("feedback", "Leaders delegate code", 1_000)),
        personal("security-boundary.md", &with_front("feedback", "Security boundary", 1_000)),
        personal("front-door-state-old.md", &with_front("project", "Old snapshot", 50_000)),
    ];
    entries.insert(0, global(&"G".repeat(20_000)));
    let fitted = fit(entries, 0, false, &|_| None);
    assert!(fitted.skipped.is_empty(), "everything fits once the snapshot is capped: {:?}", fitted.skipped.iter().map(|e| &e.name).collect::<Vec<_>>());
    let names: Vec<&str> = fitted.kept.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, ["g", "front-door-state-pm.md", "leader-delegates-code.md", "security-boundary.md", "front-door-state-old.md"]);
    let old = fitted.kept.last().unwrap();
    assert!(old.text.chars().count() < FILE_CAP_CHARS + 200 && old.text.contains("[cut:"));
}

#[test]
fn what_does_not_fit_is_left_out_whole_and_named_with_its_description() {
    // 12 files of ~8 KB (96 KB) cannot all ride in 72 KB.
    let mut entries = vec![global(&"G".repeat(10_000))];
    for i in 0..12 {
        entries.push(personal(&format!("f{i:02}.md"), &with_front("project", &format!("File number {i}"), 8_000)));
    }
    let fitted = fit(entries, 0, false, &|_| None);
    assert!(!fitted.skipped.is_empty() && fitted.skipped.len() < 12, "{}", fitted.skipped.len());
    let first_skipped = &fitted.skipped[0].name;
    assert!(fitted.kept.iter().all(|e| e.name < *first_skipped || e.tier == Tier::Global), "the newest (earliest in the list) are the ones kept");
    let tail = fitted.tail.clone().unwrap();
    for e in &fitted.skipped {
        assert!(tail.contains(&e.name), "{tail}");
        assert!(tail.contains("File number"), "{tail}");
    }
    assert!(tail.starts_with(&format!("[AgentMux] {} Personal Memory files were not sent here", fitted.skipped.len())), "{tail}");

    // And the whole composed delivery, tail included, goes out uncut.
    let composed = compose_items_tail(&fitted.kept, Reason::Compact, None, false, fitted.tail.as_deref()).unwrap();
    let total = composed.text.chars().count();
    let (_, delivered) = split_into_parts_counted(&composed.text, MAX_PART_CHARS, HOOK_PARTS);
    assert_eq!(delivered, total, "nothing is cut after the fit");
    assert!(composed.text.contains(&first_skipped.to_string()), "the list is in the text the model reads");
}

#[test]
fn the_running_summary_is_kept_room_for() {
    let mut entries = Vec::new();
    for i in 0..12 {
        entries.push(personal(&format!("f{i:02}.md"), &with_front("feedback", "d", 8_000)));
    }
    let without = fit(entries.clone(), 0, false, &|_| None).kept.len();
    let with = fit(entries, 20_000, false, &|_| None).kept.len();
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
    let compact = fit(entries.clone(), 0, true, &|_| Some(30));
    assert!(compact.tail.as_deref().unwrap().contains("Your Personal Memory is"));
    let startup = fit(entries, 0, false, &|_| Some(30));
    assert_eq!(startup.tail, None);
}

// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The user's own `widgets.json`, next to `settings.json` — Pane Tab contract
//! v1, Phase 6 (docs/specs/SPEC_PANE_TAB_CONTRACT_V1_2026_09_24.md §4).
//!
//! The widget bar used to come only from the `widgets.json` embedded at build
//! time, so there was nowhere to add a third-party (`ext:`) widget. This file's
//! entries are merged over the built-ins: a new key adds a widget, a built-in
//! key replaces that widget. Same load-then-watch shape as `settings.json`
//! (`config_watcher_fs`) and `browser_start_page`, on the same fs_watch pool,
//! riding the existing full-config broadcast.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use tokio::sync::broadcast;

use super::config_watcher_fs::{broadcast_full_config, resolve_settings_dir};
use super::eventbus::EventBus;
use super::fs_watch::{FsWatchEvent, FsWatchEventKind, FsWatchPool};
use super::mps::Broker;
use super::wconfig::{self, ConfigError, ConfigState, WidgetConfigType};

pub const USER_WIDGETS_FILE: &str = "widgets.json";

pub fn user_widgets_path() -> PathBuf {
    resolve_settings_dir().join(USER_WIDGETS_FILE)
}

/// The user's widgets; an absent file, or one with nothing in it but comments, is none. The file is read as
/// JSONC, like `settings.json`: comments and trailing commas are allowed. A file that does not parse is an
/// error that names its line and column.
pub fn read_user_widgets(path: &Path) -> Result<HashMap<String, WidgetConfigType>, ConfigError> {
    if !path.exists() {
        return Ok(HashMap::new());
    }
    let file = path.to_string_lossy().to_string();
    let content = std::fs::read_to_string(path)
        .map_err(|e| ConfigError { file: file.clone(), err: format!("cannot read widgets.json: {e}"), ..Default::default() })?;
    parse_user_widgets(&content, &file)
}

/// The widgets in a widgets.json's text (`read_user_widgets` without the file).
pub fn parse_user_widgets(content: &str, file: &str) -> Result<HashMap<String, WidgetConfigType>, ConfigError> {
    if wconfig::config_text_is_blank(content) {
        return Ok(HashMap::new());
    }
    wconfig::parse_config_text(content).map_err(|e| ConfigError {
        file: file.to_string(),
        err: format!("widgets.json doesn't parse, so your own widgets are not loaded: {e}"),
        line: Some(e.line() as u32),
        column: Some(e.column() as u32),
    })
}

/// The built-in widgets, parsed once.
fn builtin_widgets() -> &'static HashMap<String, WidgetConfigType> {
    static BUILTIN: OnceLock<HashMap<String, WidgetConfigType>> = OnceLock::new();
    BUILTIN.get_or_init(|| wconfig::build_default_config().widgets)
}

/// The built-ins with the user's entries merged over them.
pub fn merge_widgets(
    builtin: &HashMap<String, WidgetConfigType>,
    user: HashMap<String, WidgetConfigType>,
) -> HashMap<String, WidgetConfigType> {
    let mut merged = builtin.clone();
    merged.extend(user);
    merged
}

/// The user's widgets.json as last read, for the widget packages scan (its v1
/// `module` entries are implied packages) and for recomputing the merge when
/// a package changes.
static LAST_USER: std::sync::Mutex<Option<HashMap<String, WidgetConfigType>>> = std::sync::Mutex::new(None);

/// The v1 entries (`module` set) of the user's widgets.json as last read.
pub fn last_v1_entries() -> HashMap<String, WidgetConfigType> {
    let guard = LAST_USER.lock().unwrap_or_else(|p| p.into_inner());
    guard
        .as_ref()
        .map(|u| u.iter().filter(|(_, e)| !e.module.is_empty()).map(|(k, e)| (k.clone(), e.clone())).collect())
        .unwrap_or_default()
}

fn apply(config_watcher: &ConfigState, user: HashMap<String, WidgetConfigType>) {
    // The file read, so what it last reported is over.
    config_watcher.set_widgets_config_errors(Vec::new());
    wconfig::validate_widget_configs(&user);
    *LAST_USER.lock().unwrap_or_else(|p| p.into_inner()) = Some(user);
    if let Some(svc) = super::widget::packages::service() {
        svc.rescan(&last_v1_entries());
    }
    recompute(config_watcher);
}

/// Merge again: the built-ins, then each approved widget package's entries
/// (SPEC_USER_WIDGETS_AND_WIDGET_API_2026_10_09.md §5.4), then the user's own
/// widgets.json, which replaces a package's entry when it uses the same key.
pub fn recompute(config_watcher: &ConfigState) {
    let user = LAST_USER.lock().unwrap_or_else(|p| p.into_inner()).clone().unwrap_or_default();
    let mut base = builtin_widgets().clone();
    if let Some(svc) = super::widget::packages::service() {
        base.extend(super::widget::packages::widget_entries(&svc.list()));
    }
    config_watcher.update_widgets(merge_widgets(&base, user));
}

/// Whether a widgets.json entry opens the same pane section as `entry` (its view and `section`).
fn same_section(a: &serde_json::Value, b: &serde_json::Value) -> bool {
    let view = |e: &serde_json::Value| e.pointer("/blockdef/meta/view").cloned();
    a.get("section").is_some() && a.get("section") == b.get("section") && view(a) == view(b)
}

/// Where a section's `entry` is, or goes, in widgets.json: under `base` if that key is free or
/// already opens this section, otherwise under the first free `base-2`, `base-3`, …, so a key the
/// user reused for something else is never handed back as this section's. Returns the key and,
/// when the entry has to be added, the file's new text. An absent or empty file is `{}`; one that
/// doesn't parse is an error, so a file the user is halfway through editing is never rewritten.
pub fn section_entry_in(content: &str, base: &str, entry: serde_json::Value) -> Result<(String, Option<String>), String> {
    let widgets: serde_json::Map<String, serde_json::Value> = if wconfig::config_text_is_blank(content) {
        serde_json::Map::new()
    } else {
        wconfig::parse_config_text(content).map_err(|e| format!("widgets.json doesn't parse, so it was left alone: {e}"))?
    };
    for n in 1..=50 {
        let key = if n == 1 { base.to_string() } else { format!("{base}-{n}") };
        match widgets.get(&key) {
            Some(existing) if same_section(existing, &entry) => return Ok((key, None)),
            Some(_) => continue,
            None => {
                let text = insert_member(content, &key, &entry)?;
                return Ok((key, Some(text)));
            }
        }
    }
    Err(format!("widgets.json has no free key for {base}"))
}

/// Where the top-level object of a JSONC text ends, and what is last in it.
struct ObjectEnd {
    /// The byte of its closing brace.
    close: usize,
    /// The last thing in it that is not a comment or space, and where it ends: a comma goes after it.
    last: Option<(char, usize)>,
}

/// The closing brace of the text's top-level object, skipping strings and comments; `None` when it has none.
fn top_level_object_end(text: &str) -> Option<ObjectEnd> {
    let mut depth = 0usize;
    let mut last: Option<(char, usize)> = None;
    let mut in_string = false;
    let mut escaped = false;
    let mut chars = text.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        if in_string {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_string = false;
                last = Some(('"', i + 1));
            }
            continue;
        }
        match c {
            '/' if chars.peek().is_some_and(|&(_, n)| n == '/') => {
                while chars.next_if(|&(_, n)| n != '\n').is_some() {}
            }
            '/' if chars.peek().is_some_and(|&(_, n)| n == '*') => {
                chars.next();
                let mut prev = ' ';
                for (_, n) in chars.by_ref() {
                    if prev == '*' && n == '/' {
                        break;
                    }
                    prev = n;
                }
            }
            '"' => in_string = true,
            '{' | '[' => {
                depth += 1;
                last = Some((c, i + 1));
            }
            '}' | ']' => {
                depth = depth.saturating_sub(1);
                if depth == 0 && c == '}' {
                    return Some(ObjectEnd { close: i, last });
                }
                last = Some((c, i + 1));
            }
            c if c.is_whitespace() || c == '\u{feff}' => {}
            _ => last = Some((c, i + c.len_utf8())),
        }
    }
    None
}

/// The text of widgets.json with `key: entry` added as the last member of its object, and nothing else
/// changed: a person's comments, order and spacing stay where they are.
fn insert_member(content: &str, key: &str, entry: &serde_json::Value) -> Result<String, String> {
    use serde::Serialize;
    let mut body = Vec::new();
    let mut ser = serde_json::Serializer::with_formatter(&mut body, serde_json::ser::PrettyFormatter::with_indent(b"    "));
    entry.serialize(&mut ser).map_err(|e| e.to_string())?;
    let body = String::from_utf8(body).map_err(|e| e.to_string())?.replace('\n', "\n    ");
    let member = format!("    {}: {}", serde_json::to_string(key).map_err(|e| e.to_string())?, body);
    let Some(end) = top_level_object_end(content) else {
        // Nothing in the file but comments, or nothing at all.
        let head = content.trim_end();
        let gap = if head.is_empty() { "" } else { "\n" };
        return Ok(format!("{head}{gap}{{\n{member}\n}}\n"));
    };
    let (before, after) = content.split_at(end.close);
    let mut before = before.to_string();
    // A member that follows another needs the comma, put straight after it, before any comment of its line.
    if let Some((c, at)) = end.last {
        if c != '{' && c != ',' {
            before.insert(at, ',');
        }
    }
    let before = before.trim_end_matches([' ', '\t']);
    let gap = if before.ends_with('\n') { "" } else { "\n" };
    Ok(format!("{before}{gap}{member}\n{after}"))
}

/// Adds a section's `entry` to the user's widgets.json (`section_entry_in`) and applies the file
/// at once, so the widget bar can pin the entry as soon as this returns; the file watcher applies
/// it again a moment later, which changes nothing. Returns the entry's key. For "Pin this section
/// to the widget bar" (section-pins.ts): the bar pins only an entry that exists.
pub fn add_section_entry(
    config_watcher: &Arc<ConfigState>,
    event_bus: &Arc<EventBus>,
    base: &str,
    entry: serde_json::Value,
) -> Result<String, String> {
    static WRITE: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _one_at_a_time = WRITE.lock().unwrap_or_else(|p| p.into_inner());
    let path = user_widgets_path();
    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(format!("read widgets.json: {e}")),
    };
    let (key, text) = section_entry_in(&content, base, entry)?;
    if let Some(text) = text {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("create the settings folder: {e}"))?;
        }
        std::fs::write(&path, text).map_err(|e| format!("write widgets.json: {e}"))?;
    }
    apply(config_watcher, read_user_widgets(&path).map_err(|e| e.err)?);
    broadcast_full_config(config_watcher, event_bus);
    Ok(key)
}

pub fn load_user_widgets_from_disk(config_watcher: &ConfigState) {
    let path = user_widgets_path();
    match read_user_widgets(&path) {
        Ok(user) => {
            tracing::info!(path = %path.display(), count = user.len(), "user widgets.json loaded");
            apply(config_watcher, user);
        }
        Err(e) => {
            tracing::warn!(path = %path.display(), error = %e.err, line = ?e.line, "user widgets.json failed to load (built-ins only)");
            // Settings shows it, with the line; a client reads it with the full config.
            config_watcher.set_widgets_config_errors(vec![e]);
        }
    }
}

fn is_user_widgets_event(event: &FsWatchEvent) -> bool {
    let is_file = event.path.file_name().map(|n| n == USER_WIDGETS_FILE).unwrap_or(false);
    // Removed is excluded for the same reason config_watcher_fs gives: an
    // editor's unlink+recreate save would otherwise flash the built-ins only.
    is_file && matches!(event.kind, FsWatchEventKind::Created | FsWatchEventKind::Modified)
}

/// widgets.json changed: merge again, and tell every UI both the new config
/// and the package list, since its v1 `module` entries are packages too.
fn on_changed(config_watcher: &Arc<ConfigState>, event_bus: &Arc<EventBus>, broker: &Broker, user: HashMap<String, WidgetConfigType>) {
    apply(config_watcher, user);
    broadcast_full_config(config_watcher, event_bus);
    let packages = super::widget::packages::service().map(|s| s.list()).unwrap_or_default();
    super::widget::packages::publish(broker, &packages);
}

pub fn spawn_user_widgets_watcher(
    pool: Arc<FsWatchPool>,
    config_watcher: Arc<ConfigState>,
    event_bus: Arc<EventBus>,
    broker: Arc<Broker>,
) {
    let path = user_widgets_path();
    let Some(dir) = path.parent() else {
        return;
    };
    if !dir.exists() {
        tracing::warn!(dir = %dir.display(), "settings directory does not exist, widgets.json watcher not started");
        return;
    }

    // events() before subscribe_file(): a broadcast receiver only sees what is
    // sent after it subscribes (see spawn_settings_watcher).
    let mut events = pool.events();
    let _ = pool.subscribe_file(&path);
    tracing::info!(path = %path.display(), "fs_watch subscription active for user widgets.json");

    let watched_path = path.clone();
    tokio::spawn(async move {
        loop {
            match events.recv().await {
                Ok(ev) if is_user_widgets_event(&ev) => {}
                Ok(_) => continue,
                Err(broadcast::error::RecvError::Closed) => {
                    tracing::info!("user widgets watcher event stream closed, stopping");
                    break;
                }
                Err(broadcast::error::RecvError::Lagged(skipped)) => {
                    tracing::warn!(skipped, "user widgets watcher lagged; reloading defensively");
                }
            }
            tokio::time::sleep(Duration::from_millis(300)).await;
            while events.try_recv().is_ok() {}

            match read_user_widgets(&watched_path) {
                Ok(user) => {
                    tracing::info!(path = %watched_path.display(), "user widgets.json changed, reloading");
                    // A rescan hashes every package's files: off the async workers.
                    let (c, e, b) = (config_watcher.clone(), event_bus.clone(), broker.clone());
                    let _ = tokio::task::spawn_blocking(move || on_changed(&c, &e, &b, user)).await;
                }
                Err(e) => {
                    tracing::warn!(path = %watched_path.display(), error = %e.err, line = ?e.line, "user widgets.json reload parse error (keeping previous widgets)");
                    // The widgets stay as they were; Settings says why, with the line, and clears it when the file reads.
                    if config_watcher.set_widgets_config_errors(vec![e]) {
                        broadcast_full_config(&config_watcher, &event_bus);
                    }
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn widget(label: &str) -> WidgetConfigType {
        WidgetConfigType { label: label.to_string(), ..Default::default() }
    }

    #[test]
    fn a_change_to_widgets_json_tells_the_ui_the_package_list_too() {
        let seen = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let broker = Broker::new();
        let log = seen.clone();
        broker.add_observer(Arc::new(move |ev| log.lock().unwrap().push(ev.event.clone())));
        let config = Arc::new(ConfigState::new());
        on_changed(&config, &Arc::new(EventBus::new()), &broker, HashMap::from([("mine".to_string(), widget("Mine"))]));
        assert!(seen.lock().unwrap().iter().any(|e| e == super::super::mps::EVENT_WIDGET_PACKAGES));
    }

    #[test]
    fn a_section_entry_is_added_once_and_never_to_a_file_that_doesnt_parse() {
        let base = "section@warden-jobs";
        let entry = serde_json::json!({ "label": "Jobs", "section": "jobs", "blockdef": { "meta": { "view": "warden" } } });
        let (key, text) = section_entry_in("", base, entry.clone()).unwrap();
        let parsed: HashMap<String, WidgetConfigType> = serde_json::from_str(&text.unwrap()).unwrap();
        assert_eq!((key.as_str(), parsed[base].section.as_str()), (base, "jobs"));
        // Kept beside what the user already has, and not added twice.
        let (_, both) = section_entry_in(r#"{"mine@x":{"label":"Mine"}}"#, base, entry.clone()).unwrap();
        let both = both.unwrap();
        assert!(both.contains("mine@x") && both.contains(base));
        assert_eq!(section_entry_in(&both, base, entry.clone()).unwrap(), (base.to_string(), None));
        assert!(section_entry_in("{ half typed", base, entry).is_err());
    }

    #[test]
    fn a_section_entry_key_the_user_reused_for_something_else_is_never_handed_back() {
        let base = "section@warden-jobs";
        let entry = serde_json::json!({ "label": "Jobs", "section": "jobs", "blockdef": { "meta": { "view": "warden" } } });
        let taken = r#"{"section@warden-jobs":{"label":"Mine","blockdef":{"meta":{"view":"term"}}}}"#;
        let (key, text) = section_entry_in(taken, base, entry.clone()).unwrap();
        assert_eq!(key, "section@warden-jobs-2");
        let text = text.unwrap();
        assert!(text.contains("\"Mine\""));
        // Pinning it again finds the one just added.
        assert_eq!(section_entry_in(&text, base, entry).unwrap(), ("section@warden-jobs-2".to_string(), None));
    }

    #[test]
    fn a_missing_or_empty_file_is_no_widgets() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(USER_WIDGETS_FILE);
        assert!(read_user_widgets(&path).unwrap().is_empty());
        std::fs::write(&path, "  \n").unwrap();
        assert!(read_user_widgets(&path).unwrap().is_empty());
    }

    #[test]
    fn reads_an_ext_widget_with_its_module() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(USER_WIDGETS_FILE);
        std::fs::write(
            &path,
            r#"{"ext@hello":{"label":"Hello","module":"hello/index.js","blockdef":{"meta":{"view":"ext:hello"}}}}"#,
        )
        .unwrap();
        let user = read_user_widgets(&path).unwrap();
        assert_eq!(user["ext@hello"].module, "hello/index.js");
    }

    #[test]
    fn a_malformed_file_is_an_error_not_an_empty_set() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(USER_WIDGETS_FILE);
        std::fs::write(&path, "{ not json").unwrap();
        assert!(read_user_widgets(&path).is_err());
    }

    #[test]
    fn user_entries_add_widgets_and_replace_builtins_by_key() {
        let builtin = HashMap::from([("defwidget@a".to_string(), widget("A")), ("defwidget@b".to_string(), widget("B"))]);
        let user = HashMap::from([("defwidget@b".to_string(), widget("B2")), ("ext@c".to_string(), widget("C"))]);
        let merged = merge_widgets(&builtin, user);
        assert_eq!(merged.len(), 3);
        assert_eq!(merged["defwidget@a"].label, "A");
        assert_eq!(merged["defwidget@b"].label, "B2");
        assert_eq!(merged["ext@c"].label, "C");
    }

    #[test]
    fn the_merge_always_starts_from_the_builtins() {
        // A reload that drops a user widget must not keep it around.
        let builtin = HashMap::from([("defwidget@a".to_string(), widget("A"))]);
        let first = merge_widgets(&builtin, HashMap::from([("ext@c".to_string(), widget("C"))]));
        assert!(first.contains_key("ext@c"));
        let second = merge_widgets(&builtin, HashMap::new());
        assert!(!second.contains_key("ext@c"));
    }

    const ENTRY: &str = r#"{ "label": "Jobs", "section": "jobs", "blockdef": { "meta": { "view": "warden" } } }"#;

    fn entry() -> serde_json::Value {
        serde_json::from_str(ENTRY).unwrap()
    }

    #[test]
    fn a_file_with_comments_and_a_trailing_comma_loads_its_entry() {
        let text = "// my buttons\n{\n    /* the docs */\n    \"my-docs\": { \"label\": \"Docs\", \"module\": \"a//b\", },\n}\n";
        let user = parse_user_widgets(text, "widgets.json").unwrap();
        assert_eq!(user.len(), 1);
        assert_eq!(user["my-docs"].label, "Docs");
        // A // inside a string is text, not a comment.
        assert_eq!(user["my-docs"].module, "a//b");
    }

    #[test]
    fn a_file_with_nothing_but_comments_is_no_widgets() {
        assert!(parse_user_widgets("// nothing here yet\n/* or here */\n", "widgets.json").unwrap().is_empty());
        assert!(parse_user_widgets("\u{feff}", "widgets.json").unwrap().is_empty());
    }

    #[test]
    fn the_text_a_new_file_starts_with_parses_to_no_widgets_and_says_what_each_field_is() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../frontend/app/util/widgets-json-template.jsonc");
        let text = std::fs::read_to_string(path).unwrap();
        assert!(parse_user_widgets(&text, "widgets.json").unwrap().is_empty());
        for field in [
            "label",
            "icon",
            "color",
            "description",
            "display:order",
            "display:pinned",
            "blockdef",
            "singleton",
            "scope",
            "children",
            "section",
            "module",
        ] {
            assert!(text.contains(&format!("\"{field}\"")), "the text says nothing about {field}");
        }
        // The example, with its comment marks taken off, is an entry that loads.
        let live: Vec<String> = text
            .lines()
            // The example is the commented lines inside the object, indented; the field list above it starts at column 0.
            .map(|l| l.strip_prefix("    // ").unwrap_or(l).to_string())
            .collect();
        let user = parse_user_widgets(&live.join("\n"), "widgets.json").unwrap();
        assert_eq!(user["defwidget@my-docs"].label, "Docs");
    }

    #[test]
    fn a_file_that_does_not_parse_names_its_line_and_column() {
        let err = parse_user_widgets("{\n    \"a\": { \"label\": \"A\" },\n    \"b\": oops,\n}\n", "C:/x/widgets.json").unwrap_err();
        assert_eq!((err.line, err.file.as_str()), (Some(3), "C:/x/widgets.json"));
        assert!(err.column.is_some());
        assert!(err.err.contains("line 3"), "{}", err.err);
        // Comments above it do not move the line the mistake is reported on.
        let err = parse_user_widgets("// one\n// two\n{ \"a\": oops }", "widgets.json").unwrap_err();
        assert_eq!(err.line, Some(3));
    }

    #[test]
    fn adding_an_entry_keeps_every_comment_and_the_order_of_the_file() {
        let text = "// my buttons\n{\n    /* first */\n    \"mine\": { \"label\": \"M // }\" } // the only one\n}\n";
        let (key, out) = section_entry_in(text, "section@warden-jobs", entry()).unwrap();
        let out = out.unwrap();
        for kept in ["// my buttons", "/* first */", "\"M // }\"", "// the only one"] {
            assert!(out.contains(kept), "lost {kept:?} in:\n{out}");
        }
        assert!(out.find("\"mine\"").unwrap() < out.find(&key).unwrap());
        let user = parse_user_widgets(&out, "widgets.json").unwrap();
        assert_eq!((user["mine"].label.as_str(), user[&key].section.as_str()), ("M // }", "jobs"));
    }

    #[test]
    fn adding_an_entry_works_in_every_shape_of_file() {
        for text in [
            "",
            "// only a note\n",
            "{}",
            "{\n}\n",
            "{\n    // nothing yet\n}\n",
            "{ \"a\": { \"label\": \"A\" }, }",
            "{ \"a\": { \"label\": \"A\" } }",
            "{ \"a\": { \"label\": \"A\" } /* last */ }",
        ] {
            let (key, out) = section_entry_in(text, "section@x", entry()).unwrap();
            let out = out.unwrap();
            let user = parse_user_widgets(&out, "widgets.json")
                .unwrap_or_else(|e| panic!("{text:?} gave a file that does not parse ({}):\n{out}", e.err));
            assert!(user.contains_key(&key), "{text:?}");
            assert_eq!(user.len(), if text.contains("\"a\"") { 2 } else { 1 }, "{text:?}");
        }
    }

    #[test]
    fn a_file_that_does_not_parse_is_left_alone_by_the_writer() {
        assert!(section_entry_in("{ \"a\": oops // note\n}", "section@x", entry()).is_err());
    }

    #[test]
    fn a_widgets_json_error_stays_until_the_file_reads_and_does_not_clear_settings_errors() {
        let config = ConfigState::new();
        let settings = ConfigError { file: "C:/x/settings.json".into(), err: "bad".into(), ..Default::default() };
        config.set_config_errors(vec![settings.clone()]);
        let bad = parse_user_widgets("{ oops", "C:/x/widgets.json").unwrap_err();
        assert!(config.set_widgets_config_errors(vec![bad.clone()]));
        assert_eq!(config.get_full_config().config_errors, vec![settings.clone(), bad.clone()]);
        // A settings reload that reports nothing leaves widgets.json's error.
        config.set_config_errors(Vec::new());
        assert_eq!(config.get_full_config().config_errors, vec![bad]);
        config.set_config_errors(vec![settings.clone()]);
        // Applying a file that read clears its own error and not the settings'.
        apply(&config, HashMap::new());
        assert_eq!(config.get_full_config().config_errors, vec![settings]);
    }
}

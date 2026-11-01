// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Writing `settings.json` without losing what the person put in it
//! (docs/specs/SPEC_SETTINGS_JSON_EVERY_KEY_ONE_TO_ONE_2026_11_01.md §3, §5.4).
//!
//! The launcher's startup seed, srv's setting changes and the migrations all rewrite the file
//! from the template plus the person's values. They used to read the file leniently (a file that
//! did not parse read as "no settings") and write the result, so a half-pasted file, a missing
//! comma or a byte-order mark replaced everything the person had set with the bare template. Here
//! a file that cannot be read is never written, a write is atomic, and writers do not interleave.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde_json::{Map, Value};

use crate::settings_jsonc::strip_trailing_commas;

/// Settings files are a few KiB. Anything past this is not one, and is left alone.
pub const MAX_SETTINGS_BYTES: u64 = 1024 * 1024;

/// How many times a write is redone because the file changed between its read and its rename.
const MAX_REBASES: usize = 3;

static WRITE_LOCK: Mutex<()> = Mutex::new(());

/// Why a settings file could not be read. The file is left exactly as it is.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ReadError {
    #[error("settings.json can't be read: {0}")]
    Io(String),
    #[error("settings.json is larger than 1 MiB, so it is not a settings file")]
    TooLarge,
    #[error("settings.json is not a text file (it has binary content or is not valid UTF-8)")]
    NotText,
    #[error("settings.json has an error at line {line}, column {column}: {message}")]
    Syntax { line: usize, column: usize, message: String },
    #[error("settings.json must hold one JSON object")]
    NotObject,
}

/// Why a write did not happen. The file is left as it was.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum UpdateError {
    #[error("{0}")]
    Read(#[from] ReadError),
    #[error("settings.json is saved as UTF-16; save it as UTF-8 and try again")]
    Utf16,
    #[error("settings.json could not be written: {0}")]
    Write(String),
    #[error("settings.json kept changing while it was being saved; try again")]
    Conflict,
}

/// A settings file as read: its values, and the byte-level habits to write back.
#[derive(Debug, Clone, PartialEq)]
pub struct Loaded {
    pub values: Map<String, Value>,
    /// The file starts with a UTF-8 byte-order mark.
    pub bom: bool,
    /// Most of its line ends are CRLF.
    pub crlf: bool,
    /// It is UTF-16 (read, never written).
    pub utf16: bool,
}

fn decode(bytes: &[u8]) -> Result<(String, bool, bool), ReadError> {
    if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return String::from_utf8(rest.to_vec()).map(|s| (s, true, false)).map_err(|_| ReadError::NotText);
    }
    let utf16 = |rest: &[u8], le: bool| -> Result<String, ReadError> {
        if rest.len() % 2 != 0 {
            return Err(ReadError::NotText);
        }
        let units: Vec<u16> = rest
            .chunks_exact(2)
            .map(|p| if le { u16::from_le_bytes([p[0], p[1]]) } else { u16::from_be_bytes([p[0], p[1]]) })
            .collect();
        String::from_utf16(&units).map_err(|_| ReadError::NotText)
    };
    if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        return utf16(rest, true).map(|s| (s, false, true));
    }
    if let Some(rest) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        return utf16(rest, false).map(|s| (s, false, true));
    }
    if bytes.contains(&0) {
        return Err(ReadError::NotText);
    }
    String::from_utf8(bytes.to_vec()).map(|s| (s, false, false)).map_err(|_| ReadError::NotText)
}

/// `bytes` as a settings file: JSONC (comments and trailing commas allowed), one object. An empty
/// or blank file holds no settings.
pub fn parse(bytes: &[u8]) -> Result<Loaded, ReadError> {
    if bytes.len() as u64 > MAX_SETTINGS_BYTES {
        return Err(ReadError::TooLarge);
    }
    let (text, bom, utf16) = decode(bytes)?;
    let crlf = {
        let crlf = text.matches("\r\n").count();
        crlf > 0 && crlf * 2 >= text.matches('\n').count()
    };
    if text.trim().is_empty() {
        return Ok(Loaded { values: Map::new(), bom, crlf, utf16 });
    }
    // `StripComments` blanks comments and `strip_trailing_commas` blanks commas, neither moves a
    // line break, so a position in the cleaned text is the position in the file.
    let mut stripped = Vec::new();
    std::io::Read::read_to_end(&mut json_comments::StripComments::new(text.as_bytes()), &mut stripped)
        .map_err(|e| ReadError::Io(e.to_string()))?;
    let cleaned = strip_trailing_commas(&String::from_utf8_lossy(&stripped));
    match serde_json::from_str::<Value>(&cleaned) {
        Ok(Value::Object(values)) => Ok(Loaded { values, bom, crlf, utf16 }),
        Ok(_) => Err(ReadError::NotObject),
        Err(e) => Err(ReadError::Syntax { line: e.line(), column: e.column(), message: e.to_string() }),
    }
}

/// The settings file at `path`: `Ok(None)` when there is none.
pub fn read(path: &Path) -> Result<Option<Loaded>, ReadError> {
    read_bytes(path)?.map(|bytes| parse(&bytes)).transpose()
}

fn read_bytes(path: &Path) -> Result<Option<Vec<u8>>, ReadError> {
    match std::fs::metadata(path) {
        Ok(meta) if meta.len() > MAX_SETTINGS_BYTES => return Err(ReadError::TooLarge),
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(ReadError::Io(e.to_string())),
    }
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(ReadError::Io(e.to_string())),
    }
}

/// Merge user settings into a JSONC template string.
///
/// For each user key:
/// - If the key exists as a commented-out line in the template (`// "key": ...`), that line is
///   replaced with the uncommented user value.
/// - If the key is NOT in the template, it is appended before the closing `}`.
///
/// The result is always a valid JSONC file with the full template structure intact.
pub fn merge_into_template(template: &str, user_settings: &Map<String, Value>) -> String {
    if user_settings.is_empty() {
        return template.to_string();
    }

    let mut remaining: std::collections::HashMap<&str, &Value> =
        user_settings.iter().map(|(k, v)| (k.as_str(), v)).collect();
    let mut lines: Vec<String> = Vec::new();

    for line in template.lines() {
        if let Some(key) = extract_commented_setting_key(line) {
            if let Some(value) = remaining.remove(key) {
                let indent: String = line.chars().take_while(|c| c.is_whitespace()).collect();
                let val_str = serde_json::to_string(value).unwrap_or_default();
                lines.push(format!("{}\"{}\": {},", indent, key, val_str));
                continue;
            }
        }
        lines.push(line.to_string());
    }

    if !remaining.is_empty() {
        if let Some(brace_pos) = lines.iter().rposition(|l| l.trim() == "}") {
            let mut extra: Vec<String> = vec![String::new(), "    // -- User Overrides --".to_string()];
            let mut sorted_keys: Vec<&&str> = remaining.keys().collect();
            sorted_keys.sort();
            for key in sorted_keys {
                let val_str = serde_json::to_string(remaining[*key]).unwrap_or_default();
                extra.push(format!("    \"{}\": {},", key, val_str));
            }
            for (i, line) in extra.into_iter().enumerate() {
                lines.insert(brace_pos + i, line);
            }
        }
    }

    let mut result = lines.join("\n");
    if !result.ends_with('\n') {
        result.push('\n');
    }
    result
}

/// The settings key of a commented-out template line, e.g. `    // "some:key": value,`.
fn extract_commented_setting_key(line: &str) -> Option<&str> {
    let rest = line.trim_start().strip_prefix("//")?.trim_start().strip_prefix('"')?;
    let end = rest.find('"')?;
    Some(&rest[..end])
}

/// Write `contents` to `path` so a reader sees the old file or the new one, never a short one: a
/// temporary file in the same folder, flushed, then renamed over it.
pub fn write_atomic(path: &Path, contents: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let dir = path.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or_else(|| Path::new("."));
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "settings".into());
    let tmp: PathBuf = dir.join(format!(".{name}.{}.tmp", std::process::id()));
    let result = (|| {
        let mut file = std::fs::File::create(&tmp)?;
        file.write_all(contents)?;
        file.sync_all()?;
        drop(file);
        rename_over(&tmp, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

/// Rename, retrying briefly: on Windows an editor or a scanner holding the target fails it.
fn rename_over(from: &Path, to: &Path) -> std::io::Result<()> {
    let mut last = None;
    for attempt in 0..5u64 {
        match std::fs::rename(from, to) {
            Ok(()) => return Ok(()),
            Err(e) => {
                last = Some(e);
                std::thread::sleep(std::time::Duration::from_millis(20 * (attempt + 1)));
            }
        }
    }
    Err(last.expect("at least one attempt"))
}

/// The text of the settings file for `values`: the template with those values in it, in the
/// habits (byte-order mark, line ends) of the file it replaces.
fn render(template: &str, values: &Map<String, Value>, habits: Option<&Loaded>) -> Vec<u8> {
    let mut text = merge_into_template(template, values);
    if habits.is_some_and(|h| h.crlf) {
        text = text.replace("\r\n", "\n").replace('\n', "\r\n");
    }
    let mut out = Vec::new();
    if habits.is_some_and(|h| h.bom) {
        out.extend_from_slice(&[0xEF, 0xBB, 0xBF]);
    }
    out.extend_from_slice(text.as_bytes());
    out
}

/// Read the settings at `path`, let `change` edit the values, and write the file as `template`
/// with those values in it.
///
/// Nothing is written when the file cannot be read: a file with a mistake in it is the person's
/// to fix, not ours to replace (`UpdateError::Read` says where the mistake is). Writers in this
/// process take turns. When the file changes between the read and the rename (a hand edit, the
/// launcher), the read is taken again and `change` is applied to the new values, so both edits
/// land. A missing file is created. Nothing is written when the result is the file's own bytes.
pub fn update(
    path: &Path,
    template: &str,
    mut change: impl FnMut(&mut Map<String, Value>),
) -> Result<(), UpdateError> {
    let _turn = WRITE_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).map_err(|e| UpdateError::Write(e.to_string()))?;
    }
    for _ in 0..MAX_REBASES {
        let before = read_bytes(path)?;
        let loaded = before.as_deref().map(parse).transpose()?;
        if loaded.as_ref().is_some_and(|l| l.utf16) {
            return Err(UpdateError::Utf16);
        }
        let mut values = loaded.as_ref().map(|l| l.values.clone()).unwrap_or_default();
        change(&mut values);
        let out = render(template, &values, loaded.as_ref());
        if before.as_deref() == Some(out.as_slice()) {
            return Ok(());
        }
        if read_bytes(path)? != before {
            continue;
        }
        return write_atomic(path, &out).map_err(|e| UpdateError::Write(e.to_string()));
    }
    Err(UpdateError::Conflict)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const TEMPLATE: &str = "// AgentMux settings\n{\n    // -- Terminal --\n    // \"term:fontsize\": 15,\n    // \"term:scrollback\": 2000,\n}\n";

    fn set(key: &'static str, value: Value) -> impl FnMut(&mut Map<String, Value>) {
        move |m| {
            m.insert(key.to_string(), value.clone());
        }
    }

    fn file(contents: &[u8]) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(&path, contents).unwrap();
        (dir, path)
    }

    #[test]
    fn a_file_with_a_mistake_in_it_is_never_replaced() {
        for broken in [
            "{\n  \"term:fontsize\": 20\n  \"term:scrollback\": 500\n}\n", // a missing comma
            "{\n  \"term:fontsize\": 20,\n",                               // cut off in the middle of a paste
            "{ {\n  \"term:fontsize\": 20 } }\n",                          // a file pasted into a file
        ] {
            let (_dir, path) = file(broken.as_bytes());
            let err = update(&path, TEMPLATE, set("term:scrollback", json!(1))).unwrap_err();
            assert!(matches!(err, UpdateError::Read(ReadError::Syntax { .. })), "{broken:?}: {err:?}");
            assert_eq!(std::fs::read(&path).unwrap(), broken.as_bytes(), "{broken:?} must be left as it is");
        }
    }

    #[test]
    fn a_mistake_is_reported_with_its_line_and_column() {
        let (_dir, path) = file(b"{\n  // note\n  \"a\": 1\n  \"b\": 2\n}\n");
        let Err(ReadError::Syntax { line, .. }) = read(&path) else { panic!("expected a syntax error") };
        assert_eq!(line, 4, "the missing comma is found at the next member, on the file's own line");
    }

    #[test]
    fn a_file_that_is_not_one_object_is_left_alone() {
        for (text, want) in [("[1, 2]\n", ReadError::NotObject), ("\"hello\"", ReadError::NotObject)] {
            let (_dir, path) = file(text.as_bytes());
            assert_eq!(read(&path).unwrap_err(), want);
            assert!(update(&path, TEMPLATE, set("term:fontsize", json!(18))).is_err());
            assert_eq!(std::fs::read(&path).unwrap(), text.as_bytes());
        }
    }

    #[test]
    fn a_byte_order_mark_reads_and_is_kept() {
        let mut bytes = vec![0xEF, 0xBB, 0xBF];
        bytes.extend_from_slice(b"{\n  \"term:fontsize\": 20,\n}\n");
        let (_dir, path) = file(&bytes);
        assert_eq!(read(&path).unwrap().unwrap().values["term:fontsize"], json!(20));
        update(&path, TEMPLATE, set("term:scrollback", json!(500))).unwrap();
        let after = std::fs::read(&path).unwrap();
        assert!(after.starts_with(&[0xEF, 0xBB, 0xBF]), "the mark stays");
        let loaded = parse(&after).unwrap();
        assert_eq!((loaded.values["term:fontsize"].clone(), loaded.values["term:scrollback"].clone()), (json!(20), json!(500)));
    }

    #[test]
    fn crlf_line_ends_stay_crlf() {
        let (_dir, path) = file(b"{\r\n  \"term:fontsize\": 20,\r\n}\r\n");
        update(&path, TEMPLATE, set("term:scrollback", json!(500))).unwrap();
        let after = String::from_utf8(std::fs::read(&path).unwrap()).unwrap();
        assert!(after.contains("\r\n"));
        assert!(!after.replace("\r\n", "").contains('\n'), "no bare LF in a CRLF file");
    }

    #[test]
    fn a_missing_file_is_created_from_the_template() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("settings.json");
        update(&path, TEMPLATE, set("term:fontsize", json!(18))).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("\"term:fontsize\": 18,") && text.contains("// \"term:scrollback\""));
    }

    #[test]
    fn an_empty_file_holds_no_settings() {
        let (_dir, path) = file(b"  \n");
        update(&path, TEMPLATE, set("term:fontsize", json!(18))).unwrap();
        assert!(std::fs::read_to_string(&path).unwrap().contains("\"term:fontsize\": 18,"));
    }

    #[test]
    fn a_file_that_is_not_text_or_is_too_big_is_left_alone() {
        let mut huge = b"{\n".to_vec();
        huge.extend(std::iter::repeat(b' ').take(MAX_SETTINGS_BYTES as usize));
        huge.extend_from_slice(b"}\n");
        for (bytes, want) in [
            (vec![0u8, 1, 2, 0, 255], ReadError::NotText),
            (vec![b'{', 0xC3, 0x28, b'}'], ReadError::NotText),
            (huge, ReadError::TooLarge),
        ] {
            let (_dir, path) = file(&bytes);
            assert_eq!(read(&path).unwrap_err(), want);
            assert!(update(&path, TEMPLATE, set("term:fontsize", json!(18))).is_err());
            assert_eq!(std::fs::read(&path).unwrap(), bytes);
        }
    }

    #[test]
    fn a_utf16_file_reads_but_is_not_written() {
        let mut bytes = vec![0xFF, 0xFE];
        for unit in "{\"term:fontsize\": 20}".encode_utf16() {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
        let (_dir, path) = file(&bytes);
        assert_eq!(read(&path).unwrap().unwrap().values["term:fontsize"], json!(20));
        assert_eq!(update(&path, TEMPLATE, set("term:scrollback", json!(1))).unwrap_err(), UpdateError::Utf16);
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
    }

    #[test]
    fn a_hand_edit_made_while_saving_is_not_lost() {
        let (_dir, path) = file(b"{\n  \"term:fontsize\": 20,\n}\n");
        let mut hand_edit = Some(path.clone());
        update(&path, TEMPLATE, |values| {
            values.insert("term:scrollback".into(), json!(500));
            // The person saves their editor between our read and our rename.
            if let Some(p) = hand_edit.take() {
                std::fs::write(p, "{\n  \"term:fontsize\": 20,\n  \"window:zoom\": 1.5,\n}\n").unwrap();
            }
        })
        .unwrap();
        let values = read(&path).unwrap().unwrap().values;
        assert_eq!(values["window:zoom"], json!(1.5), "their edit");
        assert_eq!(values["term:scrollback"], json!(500), "ours");
    }

    #[test]
    fn writers_in_a_process_take_turns() {
        let (_dir, path) = file(b"{\n}\n");
        let path = std::sync::Arc::new(path);
        let threads: Vec<_> = (0..8)
            .map(|i| {
                let path = path.clone();
                std::thread::spawn(move || {
                    update(&path, TEMPLATE, |m| {
                        m.insert(format!("custom:k{i}"), json!(i));
                    })
                    .unwrap()
                })
            })
            .collect();
        threads.into_iter().for_each(|t| t.join().unwrap());
        let values = read(&path).unwrap().unwrap().values;
        assert_eq!((0..8).filter(|i| values.get(&format!("custom:k{i}")) == Some(&json!(i))).count(), 8);
    }

    #[test]
    fn a_write_that_fails_leaves_the_file_whole_and_no_temporary_file() {
        let (dir, path) = file(b"{\n  \"term:fontsize\": 20,\n}\n");
        let before = std::fs::read(&path).unwrap();
        // The target is a folder now: the rename cannot replace it.
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path).unwrap();
        assert!(write_atomic(&path, b"new").is_err());
        let leftovers: Vec<_> = std::fs::read_dir(dir.path()).unwrap().filter_map(|e| e.ok()).collect();
        assert_eq!(leftovers.len(), 1, "only the folder: {leftovers:?}");
        assert!(before.starts_with(b"{"));
    }

    #[test]
    fn writing_the_same_bytes_again_changes_nothing() {
        let (_dir, path) = file(TEMPLATE.as_bytes());
        update(&path, TEMPLATE, set("term:fontsize", json!(18))).unwrap();
        let first = std::fs::metadata(&path).unwrap().modified().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(30));
        update(&path, TEMPLATE, set("term:fontsize", json!(18))).unwrap();
        assert_eq!(std::fs::metadata(&path).unwrap().modified().unwrap(), first, "no rewrite");
    }

    #[test]
    fn merge_puts_a_value_where_its_commented_default_was() {
        let mut values = Map::new();
        values.insert("term:fontsize".into(), json!(18));
        values.insert("custom:key".into(), json!("x"));
        let out = merge_into_template(TEMPLATE, &values);
        assert!(out.contains("    \"term:fontsize\": 18,"));
        assert!(out.contains("// \"term:scrollback\""));
        assert!(out.contains("// -- User Overrides --") && out.contains("\"custom:key\": \"x\""));
        assert_eq!(merge_into_template(TEMPLATE, &Map::new()), TEMPLATE);
    }
}

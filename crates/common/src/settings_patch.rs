// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Changing a `settings.json` by editing its text where the change is, not by writing the template
//! again. Everything the person put there (their comments, their order, their spacing, a value
//! over several lines) is bytes this module never touches.
//!
//! The file is JSONC: one object whose members are `"key": value`, with `//` and `/* */` comments
//! between them. A commented default is a line comment alone on its line that reads `// "key": ...`.
//! (docs/specs/SPEC_SETTINGS_JSON_EVERY_KEY_ONE_TO_ONE_2026_11_01.md §5.4.) All text here has LF line ends; `settings_file` converts.

use std::ops::Range;

use serde_json::Value;

/// One top-level `"key": value` of the file.
#[derive(Debug)]
struct Member {
    key: String,
    key_start: usize,
    value: Range<usize>,
    comma: Option<usize>,
}

#[derive(Debug)]
struct Scan {
    close: usize,
    members: Vec<Member>,
    /// `//` comments between members, each from its `//` to the end of its line.
    comments: Vec<Range<usize>>,
}

fn skip_trivia(b: &[u8], mut i: usize, mut comments: Option<&mut Vec<Range<usize>>>) -> usize {
    loop {
        match b.get(i) {
            Some(c) if c.is_ascii_whitespace() => i += 1,
            Some(b'/') if b.get(i + 1) == Some(&b'/') => {
                let start = i;
                while i < b.len() && b[i] != b'\n' {
                    i += 1;
                }
                if let Some(list) = comments.as_deref_mut() {
                    list.push(start..i);
                }
            }
            Some(b'/') if b.get(i + 1) == Some(&b'*') => {
                i += 2;
                while i < b.len() && !(b[i] == b'*' && b.get(i + 1) == Some(&b'/')) {
                    i += 1;
                }
                i = (i + 2).min(b.len());
            }
            _ => return i,
        }
    }
}

/// The end of the string that starts at `i` (past its closing quote).
fn string_end(b: &[u8], mut i: usize) -> Option<usize> {
    i += 1;
    while i < b.len() {
        match b[i] {
            b'\\' => i += 2,
            b'"' => return Some(i + 1),
            _ => i += 1,
        }
    }
    None
}

fn value_end(b: &[u8], mut i: usize) -> Option<usize> {
    match b.get(i)? {
        b'"' => string_end(b, i),
        b'{' | b'[' => {
            let mut depth = 0usize;
            while i < b.len() {
                match b[i] {
                    b'"' => i = string_end(b, i)?,
                    b'/' if matches!(b.get(i + 1), Some(b'/') | Some(b'*')) => i = skip_trivia(b, i, None),
                    b'{' | b'[' => {
                        depth += 1;
                        i += 1;
                    }
                    b'}' | b']' => {
                        depth -= 1;
                        i += 1;
                        if depth == 0 {
                            return Some(i);
                        }
                    }
                    _ => i += 1,
                }
            }
            None
        }
        _ => {
            while i < b.len() && !matches!(b[i], b',' | b'}' | b']' | b'/') && !b[i].is_ascii_whitespace() {
                i += 1;
            }
            Some(i)
        }
    }
}

fn scan(text: &str) -> Option<Scan> {
    let b = text.as_bytes();
    let mut comments = Vec::new();
    let mut i = skip_trivia(b, 0, None);
    if b.get(i) != Some(&b'{') {
        return None;
    }
    i += 1;
    let mut members = Vec::new();
    loop {
        i = skip_trivia(b, i, Some(&mut comments));
        match b.get(i)? {
            b'}' => return Some(Scan { close: i, members, comments }),
            b'"' => {}
            _ => return None,
        }
        let key_start = i;
        let key_end = string_end(b, i)?;
        let key: String = serde_json::from_str(&text[key_start..key_end]).ok()?;
        i = skip_trivia(b, key_end, None);
        if b.get(i) != Some(&b':') {
            return None;
        }
        i = skip_trivia(b, i + 1, None);
        let end = value_end(b, i)?;
        let value = i..end;
        i = skip_trivia(b, end, Some(&mut comments));
        let comma = (b.get(i) == Some(&b',')).then_some(i);
        if comma.is_some() {
            i += 1;
        }
        members.push(Member { key, key_start, value, comma });
    }
}

fn line_start(text: &str, pos: usize) -> usize {
    text[..pos].rfind('\n').map_or(0, |n| n + 1)
}

/// The end of the line `pos` is on, past its line feed.
fn line_end(text: &str, pos: usize) -> usize {
    text[pos..].find('\n').map_or(text.len(), |n| pos + n + 1)
}

fn alone_on_line(text: &str, start: usize) -> bool {
    text[line_start(text, start)..start].trim().is_empty()
}

/// The key a `// "key": ...` comment is the commented default of.
fn commented_key(text: &str, comment: &Range<usize>) -> Option<String> {
    if !alone_on_line(text, comment.start) {
        return None;
    }
    let rest = text[comment.clone()].strip_prefix("//")?.trim_start();
    if !rest.starts_with('"') {
        return None;
    }
    let end = string_end(rest.as_bytes(), 0)?;
    if !rest[end..].trim_start().starts_with(':') {
        return None;
    }
    serde_json::from_str(&rest[..end]).ok()
}

fn is_header(line: &str) -> bool {
    line.trim_start().starts_with("// --")
}

/// The lines of the commented default at `comment`, with the plain comment lines right above it
/// (its description).
fn entry_lines(text: &str, comment: &Range<usize>, scan: &Scan) -> Range<usize> {
    let end = line_end(text, comment.end);
    let mut start = line_start(text, comment.start);
    while start > 0 {
        let prev = line_start(text, start - 1);
        let line = &text[prev..start];
        let is_plain_comment = line.trim_start().starts_with("//")
            && !is_header(line)
            && !scan.comments.iter().any(|c| c.start >= prev && c.start < start && commented_key(text, c).is_some());
        if !is_plain_comment {
            break;
        }
        start = prev;
    }
    start..end
}

fn commented(text: &str, scan: &Scan, key: &str) -> Option<Range<usize>> {
    scan.comments.iter().find(|c| commented_key(text, c).as_deref() == Some(key)).cloned()
}

/// A value as it is written: short ones on a line; objects and lists that are long or were
/// already over several lines, one member to a line, lined up with the key they belong to.
fn render_value(value: &Value, indent: &str, was_multiline: bool) -> String {
    let compact = serde_json::to_string(value).unwrap_or_default();
    if !matches!(value, Value::Object(m) if !m.is_empty()) && !matches!(value, Value::Array(a) if !a.is_empty()) {
        return compact;
    }
    if !was_multiline && compact.len() <= 72 {
        return compact;
    }
    let pretty = serde_json::to_string_pretty(value).unwrap_or(compact);
    pretty.replace('\n', &format!("\n{indent}"))
}

fn indent_of(text: &str, pos: usize) -> String {
    text[line_start(text, pos)..pos].chars().take_while(|c| c.is_whitespace()).collect()
}

/// `text` with `key` set to `value`: in place when it is set, in place of its commented default
/// when it has one, otherwise last in the file.
pub(crate) fn set(text: &str, key: &str, value: &Value) -> Option<String> {
    let found = scan(text)?;
    if let Some(member) = found.members.iter().rev().find(|m| m.key == key) {
        let old = &text[member.value.clone()];
        let rendered = render_value(value, &indent_of(text, member.key_start), old.contains('\n'));
        return Some(format!("{}{}{}", &text[..member.value.start], rendered, &text[member.value.end..]));
    }
    let key_text = serde_json::to_string(key).ok()?;
    if let Some(comment) = commented(text, &found, key) {
        let indent = indent_of(text, comment.start);
        let line = format!("{indent}{key_text}: {},", render_value(value, &indent, false));
        let mut out = format!("{}{}{}", &text[..comment.start - indent.len()], line, &text[comment.end..]);
        // The member before it, when it was the last one, has no comma yet.
        if let Some(before) = found.members.iter().filter(|m| m.value.end <= comment.start).last() {
            if before.comma.is_none() {
                out.insert(before.value.end, ',');
            }
        }
        return Some(out);
    }
    // Last in the file, under a heading, with the comma the member before it may lack.
    let mut out = text.to_string();
    let mut at = found.close;
    let mut insert = String::new();
    if let Some(last) = found.members.last().filter(|m| m.comma.is_none()) {
        out.insert(last.value.end, ',');
        at += 1;
    }
    let heading = "// -- User Overrides --";
    let alone = alone_on_line(&out, at);
    if alone {
        at = line_start(&out, at);
    } else {
        insert.push('\n');
    }
    if !text.contains(heading) {
        insert.push_str(&format!("\n    {heading}\n"));
    }
    insert.push_str(&format!("    {key_text}: {},\n", render_value(value, "    ", false)));
    if !alone {
        insert.push('\n');
    }
    out.insert_str(at, &insert);
    Some(out)
}

/// `text` without `key`: its lines go, comments above them stay.
pub(crate) fn remove(text: &str, key: &str) -> Option<String> {
    let mut out = text.to_string();
    while let Some(member) = scan(&out)?.members.into_iter().rev().find(|m| m.key == key) {
        let tail = member.comma.map_or(member.value.end, |c| c + 1);
        let from = line_start(&out, member.key_start);
        let to = line_end(&out, tail);
        let whole_lines = out[from..member.key_start].trim().is_empty()
            && out[tail..to].trim().is_empty()
            || out[tail..to].trim_start().starts_with("//") && out[from..member.key_start].trim().is_empty();
        let range = if whole_lines { from..to } else { member.key_start..tail };
        out.replace_range(range, "");
    }
    Some(out)
}

/// `text` with `key` back at its default: the member's lines become the template's commented
/// default for it, where they were. A key the template does not list is removed.
pub(crate) fn reset(text: &str, key: &str, template: &str) -> Option<String> {
    let tscan = scan(template)?;
    let Some(line) = commented(template, &tscan, key).map(|c| template[c].to_string()) else {
        return remove(text, key);
    };
    let mut out = text.to_string();
    while let Some(member) = scan(&out)?.members.into_iter().rev().find(|m| m.key == key) {
        let tail = member.comma.map_or(member.value.end, |c| c + 1);
        let from = line_start(&out, member.key_start);
        let to = line_end(&out, tail);
        let whole_lines = out[from..member.key_start].trim().is_empty() && {
            let rest = out[tail..to].trim();
            rest.is_empty() || rest.starts_with("//")
        };
        if !whole_lines {
            return remove(text, key);
        }
        let indent = indent_of(&out, member.key_start);
        out.replace_range(from..to, &format!("{indent}{line}\n"));
    }
    Some(out)
}

/// The end of the line a member ends on, past any block comment that starts on that line, however
/// many lines it runs: text inserted there is never inside a comment.
fn end_of_line_after(text: &str, pos: usize) -> usize {
    let b = text.as_bytes();
    let mut i = pos;
    loop {
        while matches!(b.get(i), Some(b' ') | Some(b'\t') | Some(b'\r')) {
            i += 1;
        }
        if b.get(i) == Some(&b'/') && b.get(i + 1) == Some(&b'*') {
            i = text[i + 2..].find("*/").map_or(text.len(), |n| i + 2 + n + 2);
            continue;
        }
        return if i >= text.len() { text.len() } else { line_end(text, i) };
    }
}

/// Where a key sits in the file, for placing a new neighbour after it: the end of its line.
fn end_of_key(text: &str, scan: &Scan, key: &str) -> Option<usize> {
    if let Some(member) = scan.members.iter().rev().find(|m| m.key == key) {
        let mut at = end_of_line_after(text, member.comma.map_or(member.value.end, |c| c + 1));
        // A member after it on the same line may run on over later lines: go past its value too.
        while let Some(inside) = scan.members.iter().find(|m| m.value.start < at && at < m.value.end) {
            at = end_of_line_after(text, inside.comma.map_or(inside.value.end, |c| c + 1));
        }
        return Some(at);
    }
    commented(text, scan, key).map(|c| line_end(text, c.end))
}

/// The heading line at `at`. The plain comments under it belong to the first entry of its section.
fn header_block(text: &str, at: usize) -> Range<usize> {
    line_start(text, at)..line_end(text, at)
}

/// Bring the commented defaults of `text` to `template`'s: a commented default whose text changed
/// is rewritten, a key the file lacks is added in its section. Keys the person set, their own
/// comments and everything else stay as they are.
pub(crate) fn refresh(text: &str, template: &str) -> Option<String> {
    let tscan = scan(template)?;
    let mut out = text.to_string();
    let mut section: Option<(String, Range<usize>)> = None; // heading text, heading lines in the template
    let mut previous: Option<String> = None;
    let mut pending: Vec<(String, Range<usize>)> = Vec::new(); // (key, entry lines) in template order
    let mut sections: Vec<Option<(String, Range<usize>)>> = Vec::new();
    for comment in &tscan.comments {
        if alone_on_line(template, comment.start) && is_header(&template[line_start(template, comment.start)..]) {
            let block = header_block(template, comment.start);
            section = Some((template[line_start(template, comment.start)..line_end(template, comment.start)].trim().to_string(), block));
            continue;
        }
        if let Some(key) = commented_key(template, comment) {
            pending.push((key, entry_lines(template, comment, &tscan)));
            sections.push(section.clone());
        }
    }
    for ((key, lines), head) in pending.into_iter().zip(sections) {
        let entry = &template[lines.clone()];
        let mut found = scan(&out)?;
        if !alone_on_line(&out, found.close) {
            out.insert(found.close, '\n');
            found = scan(&out)?;
        }
        if found.members.iter().any(|m| m.key == key) {
            previous = Some(key);
            continue;
        }
        if let Some(comment) = commented(&out, &found, &key) {
            // Only the key line: a note the person wrote above it, or a description from an older
            // template, stays as it is.
            if let Some(line) = commented(template, &tscan, &key) {
                if out[comment.clone()] != template[line.clone()] {
                    out.replace_range(comment, &template[line]);
                }
            }
            previous = Some(key);
            continue;
        }
        // Missing: after the key before it in its section, else under its heading, else a new section.
        let after_previous = previous.as_deref().and_then(|p| {
            let in_section = pending_in_section(&tscan, template, p, head.as_ref().map(|h| h.0.as_str()));
            in_section.then(|| end_of_key(&out, &found, p)).flatten()
        });
        let at = match (after_previous, &head) {
            (Some(at), _) => at,
            (None, Some((title, block))) => {
                let existing = found
                    .comments
                    .iter()
                    .find(|c| alone_on_line(&out, c.start) && out[line_start(&out, c.start)..line_end(&out, c.start)].trim() == title)
                    .cloned();
                match existing {
                    Some(c) => header_block(&out, c.start).end,
                    None => {
                        let heading = &template[block.clone()];
                        let mut at = line_start(&out, found.close);
                        if !alone_on_line(&out, found.close) {
                            out.insert(found.close, '\n');
                            at = found.close + 1;
                        }
                        out.insert_str(at, &format!("\n{heading}"));
                        at + 1 + heading.len()
                    }
                }
            }
            (None, None) => line_end(&out, found.close.min(out.len().saturating_sub(1))).min(line_start(&out, found.close)),
        };
        out.insert_str(at, entry);
        previous = Some(key);
    }
    Some(out)
}

/// Whether the template's key `key` sits under the heading `title`.
fn pending_in_section(tscan: &Scan, template: &str, key: &str, title: Option<&str>) -> bool {
    let Some(c) = commented(template, tscan, key) else { return false };
    let mut current: Option<String> = None;
    for comment in &tscan.comments {
        if comment.start > c.start {
            break;
        }
        if alone_on_line(template, comment.start) && is_header(&template[line_start(template, comment.start)..]) {
            current = Some(template[line_start(template, comment.start)..line_end(template, comment.start)].trim().to_string());
        }
    }
    current.as_deref() == title
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const TEMPLATE: &str = "// AgentMux settings\n{\n    // -- Terminal --\n    // \"term:fontsize\": 15,\n    // Lines kept after they scroll off.\n    // \"term:scrollback\": 2000,\n\n    // -- Window --\n    // \"window:zoom\": 1.0,\n}\n";

    #[test]
    fn a_set_value_changes_only_its_value() {
        let text = "{\n  // my note\n  \"term:fontsize\": 20,   // keep\n  \"a\": [1,\n    2],\n}\n";
        let out = set(text, "term:fontsize", &json!(22)).unwrap();
        assert_eq!(out, "{\n  // my note\n  \"term:fontsize\": 22,   // keep\n  \"a\": [1,\n    2],\n}\n");
    }

    #[test]
    fn a_commented_default_becomes_the_value_in_place() {
        let out = set(TEMPLATE, "term:scrollback", &json!(500)).unwrap();
        assert!(out.contains("    // Lines kept after they scroll off.\n    \"term:scrollback\": 500,\n"));
        assert!(out.contains("    // \"term:fontsize\": 15,\n"));
    }

    #[test]
    fn a_new_key_goes_last_under_a_heading_and_the_member_before_gets_its_comma() {
        let out = set("{\n  \"a\": 1\n}\n", "custom:key", &json!("x")).unwrap();
        assert_eq!(out, "{\n  \"a\": 1,\n\n    // -- User Overrides --\n    \"custom:key\": \"x\",\n}\n");
        let again = set(&out, "custom:two", &json!(2)).unwrap();
        assert_eq!(again.matches("User Overrides").count(), 1);
        assert!(again.ends_with("    \"custom:two\": 2,\n}\n"));
        assert!(crate::settings_file::parse(again.as_bytes()).is_ok());
    }

    #[test]
    fn a_compact_file_stays_valid_when_a_key_is_added() {
        let out = set("{\"a\": 1}", "b", &json!(2)).unwrap();
        let values = crate::settings_file::parse(out.as_bytes()).unwrap().values;
        assert_eq!((values["a"].clone(), values["b"].clone()), (json!(1), json!(2)));
    }

    #[test]
    fn a_value_over_several_lines_keeps_its_layout_until_it_changes() {
        let text = "{\n  \"connections\": {\n    \"a\": {\"ssh:hostname\": \"x\"}\n  },\n  \"term:fontsize\": 20\n}\n";
        let out = set(text, "term:fontsize", &json!(21)).unwrap();
        assert!(out.starts_with("{\n  \"connections\": {\n    \"a\": {\"ssh:hostname\": \"x\"}\n  },\n"));
        let changed = set(text, "connections", &json!({"a": {"ssh:hostname": "y"}, "b": {}})).unwrap();
        assert!(changed.contains("\"connections\": {\n    \"a\": {"), "{changed}");
        assert!(crate::settings_file::parse(changed.as_bytes()).is_ok());
    }

    #[test]
    fn removing_a_key_takes_its_lines_and_leaves_the_comments_around_it() {
        let text = "{\n  // keep me\n  \"a\": 1, // gone with it\n  \"b\": {\n    \"c\": 2\n  },\n  \"d\": 3\n}\n";
        assert_eq!(remove(text, "a").unwrap(), "{\n  // keep me\n  \"b\": {\n    \"c\": 2\n  },\n  \"d\": 3\n}\n");
        assert_eq!(remove(text, "b").unwrap(), "{\n  // keep me\n  \"a\": 1, // gone with it\n  \"d\": 3\n}\n");
        assert_eq!(remove("{\"a\": 1, \"b\": 2}", "a").unwrap(), "{ \"b\": 2}");
    }

    #[test]
    fn a_comment_that_looks_like_a_member_inside_a_string_or_block_is_not_one() {
        let text = "{\n  \"note\": \"// \\\"term:fontsize\\\": 9\",\n  /* // \"term:scrollback\": 1, */\n}\n";
        let out = set(text, "term:fontsize", &json!(18)).unwrap();
        assert!(out.contains("\"term:fontsize\": 18,") && out.contains("\"note\": \"// \\\"term:fontsize\\\": 9\""));
    }

    #[test]
    fn refresh_adds_the_missing_keys_in_their_sections_and_leaves_set_values() {
        let old = "{\n    // -- Terminal --\n    \"term:fontsize\": 20,\n    // my own note\n}\n";
        let out = refresh(old, TEMPLATE).unwrap();
        assert!(out.contains("    \"term:fontsize\": 20,\n    // Lines kept after they scroll off.\n    // \"term:scrollback\": 2000,\n"), "{out}");
        assert!(out.contains("// -- Window --") && out.contains("// \"window:zoom\": 1.0,"));
        assert!(out.contains("// my own note"));
        assert!(!out.contains("// \"term:fontsize\""), "a set key is not added again");
        assert!(crate::settings_file::parse(out.as_bytes()).is_ok());
    }

    #[test]
    fn refresh_updates_a_commented_default_and_does_nothing_twice() {
        let old = TEMPLATE.replace("2000", "1000");
        let out = refresh(&old, TEMPLATE).unwrap();
        assert_eq!(out, TEMPLATE);
        // A note above a commented default, or an older description, is not the key line: it stays.
        let noted = TEMPLATE.replace("2000", "1000").replace("Lines kept after they scroll off.", "My note: keep this.");
        let kept = refresh(&noted, TEMPLATE).unwrap();
        assert!(kept.contains("// My note: keep this.
    // \"term:scrollback\": 2000,"), "{kept}");
        assert_eq!(refresh(&out, TEMPLATE).unwrap(), out);
        assert_eq!(refresh(TEMPLATE, TEMPLATE).unwrap(), TEMPLATE);
    }

    #[test]
    fn refresh_fills_an_empty_file() {
        let out = refresh("{\n}\n", TEMPLATE).unwrap();
        for key in ["term:fontsize", "term:scrollback", "window:zoom"] {
            assert!(out.contains(&format!("// \"{key}\"")), "{key} in {out}");
        }
        assert!(crate::settings_file::parse(out.as_bytes()).is_ok());
    }

    const DESCRIBED: &str = "{
    // -- Startup --
    // Show no splash.
    // Second line.
    // \"splash:disabled\": false,
    // \"splash:other\": 1,
}
";

    #[test]
    fn a_reset_puts_the_commented_default_back_in_place_and_cycles_do_not_grow_the_file() {
        let start = DESCRIBED.to_string();
        let set_it = set(&start, "splash:disabled", &json!(true)).unwrap();
        assert!(set_it.contains("    // Show no splash.
    // Second line.
    \"splash:disabled\": true,
"));
        let back = reset(&set_it, "splash:disabled", DESCRIBED).unwrap();
        assert_eq!(back, DESCRIBED);
        assert_eq!(refresh(&back, DESCRIBED).unwrap(), DESCRIBED);
        let mut text = start;
        for _ in 0..3 {
            text = set(&text, "splash:disabled", &json!(true)).unwrap();
            text = refresh(&reset(&text, "splash:disabled", DESCRIBED).unwrap(), DESCRIBED).unwrap();
        }
        assert_eq!(text, DESCRIBED);
    }

    #[test]
    fn a_section_new_in_the_template_comes_with_its_description_once() {
        let out = refresh("{
    \"a\": 1,
}
", DESCRIBED).unwrap();
        assert_eq!(out.matches("Show no splash.").count(), 1, "{out}");
        assert_eq!(refresh(&out, DESCRIBED).unwrap(), out, "a second refresh changes nothing");
    }

    #[test]
    fn a_compact_one_line_file_gets_its_new_keys_inside_the_braces_and_stays_stable() {
        let out = refresh("{\"term:fontsize\": 20}", TEMPLATE).unwrap();
        assert!(crate::settings_file::parse(out.as_bytes()).is_ok(), "{out}");
        assert!(out.trim_end().ends_with('}') && out.contains("// \"term:scrollback\": 2000,"), "{out}");
        assert_eq!(refresh(&out, TEMPLATE).unwrap(), out);
    }

    #[test]
    fn a_note_shaped_like_a_commented_default_is_that_keys_default() {
        let text = "{
    // \"term:scrollback\": 9,
}
";
        let out = refresh(text, TEMPLATE).unwrap();
        assert!(out.contains("    // \"term:scrollback\": 2000,"), "its value is refreshed: {out}");
        assert_eq!(out.matches("term:scrollback").count(), 1);
    }

    #[test]
    fn setting_a_commented_default_after_a_last_member_with_no_comma_keeps_the_file_valid() {
        let refreshed = refresh("{\"term:fontsize\": 20}", TEMPLATE).unwrap();
        let out = set(&refreshed, "term:scrollback", &json!(500)).unwrap();
        let values = crate::settings_file::parse(out.as_bytes()).unwrap().values;
        assert_eq!((values["term:fontsize"].clone(), values["term:scrollback"].clone()), (json!(20), json!(500)), "{out}");
        let spaced = set("{
    \"a\": 1
    // \"term:scrollback\": 2000,
}
", "term:scrollback", &json!(5)).unwrap();
        assert!(crate::settings_file::parse(spaced.as_bytes()).is_ok(), "{spaced}");
    }

    #[test]
    fn a_new_key_is_never_placed_inside_a_block_comment_after_a_member() {
        let text = "{
    \"term:fontsize\": 20, /* my note
       over lines
    */
}
";
        let out = refresh(text, TEMPLATE).unwrap();
        assert!(out.contains("/* my note
       over lines
    */
    // Lines kept after they scroll off.
    // \"term:scrollback\": 2000,"), "{out}");
        assert_eq!(refresh(&out, TEMPLATE).unwrap(), out, "a second refresh changes nothing");
        assert!(crate::settings_file::parse(out.as_bytes()).is_ok());
    }

    #[test]
    fn a_new_key_is_never_placed_inside_a_value_that_starts_on_the_same_line() {
        let text = "{\n    \"term:fontsize\": 20, \"connections\": {\n        \"a\": {}\n    },\n}\n";
        let out = refresh(text, TEMPLATE).unwrap();
        let values = crate::settings_file::parse(out.as_bytes()).unwrap().values;
        assert_eq!(values["connections"], json!({"a": {}}), "{out}");
        assert!(out.find("// \"term:scrollback\"").unwrap() > out.find("    },").unwrap(), "{out}");
        assert_eq!(refresh(&out, TEMPLATE).unwrap(), out, "a second refresh changes nothing");
    }
}

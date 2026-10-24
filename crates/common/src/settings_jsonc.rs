// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Reading `settings.json` the way srv does: JSONC, so `//` and `/* */`
//! comments and trailing commas are allowed. The seeded template has both, so
//! a reader that parses strict JSON gets nothing from a seeded file (#1903).

use std::io::Read;

/// `text` as JSONC, or `None` when it isn't valid even then.
pub fn parse_jsonc(text: &str) -> Option<serde_json::Value> {
    let mut bytes = Vec::new();
    std::io::BufReader::new(json_comments::StripComments::new(text.as_bytes()))
        .read_to_end(&mut bytes)
        .ok()?;
    serde_json::from_str(&strip_trailing_commas(&String::from_utf8_lossy(&bytes))).ok()
}

/// `input` with each comma that ends an object or array blanked out, leaving
/// commas inside strings alone.
pub fn strip_trailing_commas(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    let mut in_string = false;
    let mut last_comma_pos: Option<usize> = None;

    while let Some(ch) = chars.next() {
        if in_string {
            result.push(ch);
            if ch == '\\' {
                if let Some(&next) = chars.peek() {
                    result.push(next);
                    chars.next();
                }
            } else if ch == '"' {
                in_string = false;
            }
        } else {
            match ch {
                '"' => {
                    in_string = true;
                    last_comma_pos = None;
                    result.push(ch);
                }
                ',' => {
                    last_comma_pos = Some(result.len());
                    result.push(ch);
                }
                '}' | ']' => {
                    if let Some(pos) = last_comma_pos {
                        result.replace_range(pos..pos + 1, " ");
                    }
                    last_comma_pos = None;
                    result.push(ch);
                }
                c if c.is_whitespace() => {
                    result.push(ch);
                }
                _ => {
                    last_comma_pos = None;
                    result.push(ch);
                }
            }
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_comments_and_trailing_commas() {
        let text = "{
  // The splash screen.
  \"splash:disabled\": true,
  /* tray */ \"app:runinbackground\": false,
  \"list\": [1, 2,],
}
";
        let v = parse_jsonc(text).unwrap();
        assert_eq!(v["splash:disabled"], true);
        assert_eq!(v["app:runinbackground"], false);
        assert_eq!(v["list"], serde_json::json!([1, 2]));
    }

    #[test]
    fn leaves_strings_alone_and_refuses_what_isnt_json() {
        assert_eq!(parse_jsonc(r#"{"a": "b,} // not a comment"}"#).unwrap()["a"], "b,} // not a comment");
        assert!(parse_jsonc("{nope").is_none());
    }

    #[test]
    fn strips_only_trailing_commas() {
        assert_eq!(strip_trailing_commas(r#"{"a": 1,}"#), r#"{"a": 1 }"#);
        assert_eq!(strip_trailing_commas(r#"[1, 2,]"#), r#"[1, 2 ]"#);
        assert_eq!(strip_trailing_commas(r#"{"a": "b,}"}"#), r#"{"a": "b,}"}"#);
        assert_eq!(strip_trailing_commas(r#"{"a": 1, "b": 2}"#), r#"{"a": 1, "b": 2}"#);
    }
}

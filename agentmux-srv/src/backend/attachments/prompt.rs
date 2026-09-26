// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Turning a message's attachments into what the agent receives
//! (docs/specs/SPEC_AGENT_PANE_IMAGE_ATTACHMENTS_2026_09_26.md §6.6):
//!
//! - every provider gets a numbered list of send-copy paths appended to the
//!   text, in an `<attached_images>` block, so any agent that can open a
//!   local file can see every image;
//! - Claude (stream-json) additionally gets the first images inline as
//!   `image` content blocks, within a count and size budget.
//!
//! The `<attached_images>` block is also how the pane rebuilds the
//! thumbnails on reload: each line names the attachment and ends with the
//! send-copy path, whose file name starts with the attachment id.

use std::path::PathBuf;

use base64::Engine;

use super::Service;
use crate::backend::rpc_types::AttachmentRef;

/// Inline at most this many base64 bytes in one Claude message. The API
/// caps a request at 32 MB, and the whole conversation is resent each turn.
pub const INLINE_MAX_B64_BYTES: usize = 20 * 1024 * 1024;
pub const DEFAULT_INLINE_MAX_COUNT: usize = 20;
/// Inline at most this much base64 across one Claude session. Claude Code
/// keeps every inline image in its own session transcript.
pub const DEFAULT_SESSION_INLINE_MB: u64 = 50;

pub const BLOCK_OPEN: &str = "<attached_images>";
pub const BLOCK_CLOSE: &str = "</attached_images>";

/// One attachment resolved to the file the agent gets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved {
    /// 1-based, matching the composer's tile badges.
    pub number: usize,
    pub name: String,
    pub path: PathBuf,
    pub mime: String,
}

/// Everything a turn needs from its attachments.
#[derive(Debug, Clone, Default)]
pub struct Prepared {
    /// Appended to the message text.
    pub list: String,
    /// Anthropic `image` content blocks, in order. Empty unless asked for.
    pub inline: Vec<serde_json::Value>,
}

/// Resolve refs to send-copies. Refs whose files are gone (swept, or never
/// finished processing) come back as names in the second list. Blocking.
pub fn resolve(svc: &Service, refs: &[AttachmentRef]) -> (Vec<Resolved>, Vec<String>) {
    let mut found = Vec::new();
    let mut missing = Vec::new();
    for (i, r) in refs.iter().enumerate() {
        match svc.send_path(&r.id) {
            Some((path, mime)) => found.push(Resolved {
                number: i + 1,
                name: clean_name(&r.name),
                path,
                mime,
            }),
            None => missing.push(clean_name(&r.name)),
        }
    }
    (found, missing)
}

/// Build the list and (when `inline_max_count > 0`) the inline blocks, using
/// at most `inline_budget` base64 bytes (further capped at
/// [`INLINE_MAX_B64_BYTES`] per message). Reads the send-copies from disk
/// for the inline part. Blocking.
pub fn prepare(
    items: &[Resolved],
    missing: &[String],
    inline_max_count: usize,
    inline_budget: u64,
) -> Prepared {
    let mut inline = Vec::new();
    let mut budget = (INLINE_MAX_B64_BYTES as u64).min(inline_budget) as usize;
    for item in items.iter().take(inline_max_count) {
        let Ok(bytes) = std::fs::read(&item.path) else {
            break;
        };
        let encoded = base64::engine::general_purpose::STANDARD.encode(&bytes);
        if encoded.len() > budget {
            break;
        }
        budget -= encoded.len();
        inline.push(serde_json::json!({
            "type": "image",
            "source": { "type": "base64", "media_type": item.mime, "data": encoded },
        }));
    }
    Prepared {
        list: list_block(items, missing, inline.len()),
        inline,
    }
}

/// Base64 bytes carried by inline blocks, for the session budget.
pub fn inline_bytes(blocks: &[serde_json::Value]) -> u64 {
    blocks
        .iter()
        .filter_map(|b| b.pointer("/source/data").and_then(|d| d.as_str()))
        .map(|d| d.len() as u64)
        .sum()
}

/// The `<attached_images>` block.
pub fn list_block(items: &[Resolved], missing: &[String], inline_count: usize) -> String {
    let total = items.len() + missing.len();
    let mut out = String::new();
    out.push_str(BLOCK_OPEN);
    out.push('\n');
    let noun = if total == 1 { "image" } else { "images" };
    out.push_str(&format!(
        "The user attached {total} {noun}. The numbers match how the user refers to them."
    ));
    if inline_count > 0 && inline_count == items.len() {
        out.push_str(" They are shown above; the files are listed here too.");
    } else if inline_count > 0 {
        out.push_str(&format!(
            " Images 1–{inline_count} are shown above. Open the others from these paths with your file or image viewing tool when you need them."
        ));
    } else if !items.is_empty() {
        out.push_str(
            " Open them from these paths with your file or image viewing tool when you need them.",
        );
    }
    out.push('\n');
    for item in items {
        out.push_str(&format!(
            "{}. {} — {}\n",
            item.number,
            item.name,
            item.path.display()
        ));
    }
    for name in missing {
        out.push_str(&format!("- {name} — (no longer available)\n"));
    }
    out.push_str(BLOCK_CLOSE);
    out
}

/// The user's text with the attachment list after it.
pub fn append(message: &str, list: &str) -> String {
    if list.is_empty() {
        return message.to_string();
    }
    let trimmed = message.trim_end();
    if trimmed.is_empty() {
        list.to_string()
    } else {
        format!("{trimmed}\n\n{list}")
    }
}

/// Names go into a one-line list entry; keep them on one line and short.
fn clean_name(name: &str) -> String {
    let one_line: String = name
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let trimmed = one_line.trim();
    let short: String = trimmed.chars().take(120).collect();
    if short.is_empty() {
        "image".to_string()
    } else {
        short
    }
}

/// A Claude stream-json user line. With no images, `content` is the plain
/// string it always was; with images, the image blocks come first and the
/// text last, as the Messages API recommends.
pub fn claude_user_line(message: String, images: Vec<serde_json::Value>) -> String {
    let content = if images.is_empty() {
        serde_json::Value::String(message)
    } else {
        let mut blocks = images;
        blocks.push(serde_json::json!({ "type": "text", "text": message }));
        serde_json::Value::Array(blocks)
    };
    serde_json::json!({
        "type": "user",
        "message": { "role": "user", "content": content }
    })
    .to_string()
}

/// The form of a Claude stdin line that AgentMux stores for replay: image
/// blocks dropped and the content collapsed back to its text, which still
/// holds the `<attached_images>` list. Every reader of stored user lines
/// expects `content` to be a string, and a stored line should never carry
/// megabytes of base64. Lines without an array `content` pass through.
pub fn persisted_line(json_str: &str) -> std::borrow::Cow<'_, str> {
    let Ok(mut v) = serde_json::from_str::<serde_json::Value>(json_str) else {
        return json_str.into();
    };
    let Some(content) = v.pointer_mut("/message/content") else {
        return json_str.into();
    };
    let Some(blocks) = content.as_array() else {
        return json_str.into();
    };
    let text: Vec<&str> = blocks
        .iter()
        .filter(|b| b.get("type").and_then(|t| t.as_str()) == Some("text"))
        .filter_map(|b| b.get("text").and_then(|t| t.as_str()))
        .collect();
    *content = serde_json::Value::String(text.join("\n\n"));
    v.to_string().into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(n: usize, name: &str, path: &str) -> Resolved {
        Resolved {
            number: n,
            name: name.into(),
            path: path.into(),
            mime: "image/png".into(),
        }
    }

    #[test]
    fn list_numbers_items_and_names_missing_ones() {
        let items = vec![
            item(1, "a.png", "/s/aa.v1-e2000.send.png"),
            item(3, "c.png", "/s/cc.v1-e2000.send.png"),
        ];
        let list = list_block(&items, &["b.png".into()], 0);
        assert!(list.starts_with(BLOCK_OPEN));
        assert!(list.ends_with(BLOCK_CLOSE));
        assert!(list.contains("The user attached 3 images."));
        assert!(list.contains("\n1. a.png — /s/aa.v1-e2000.send.png\n"));
        assert!(list.contains("\n3. c.png — /s/cc.v1-e2000.send.png\n"));
        assert!(list.contains("- b.png — (no longer available)"));
    }

    #[test]
    fn list_says_which_images_are_inline() {
        let items = vec![item(1, "a", "/a"), item(2, "b", "/b")];
        assert!(list_block(&items, &[], 2).contains("They are shown above"));
        assert!(list_block(&items, &[], 1).contains("Images 1–1 are shown above"));
        assert!(list_block(&items, &[], 0).contains("Open them from these paths"));
    }

    #[test]
    fn append_puts_the_list_after_the_text() {
        assert_eq!(append("look\n", "<L>"), "look\n\n<L>");
        assert_eq!(append("   ", "<L>"), "<L>");
        assert_eq!(append("hi", ""), "hi");
    }

    #[test]
    fn names_stay_on_one_line() {
        assert_eq!(clean_name("a\nb\tc.png"), "a b c.png");
        assert_eq!(clean_name("  "), "image");
        assert_eq!(clean_name(&"x".repeat(500)).chars().count(), 120);
    }

    #[test]
    fn prepare_inlines_within_count_and_reads_files() {
        let d = tempfile::tempdir().unwrap();
        let paths: Vec<_> = (0..3)
            .map(|i| {
                let p = d.path().join(format!("{i}.png"));
                std::fs::write(&p, [0x89, b'P', b'N', b'G', i]).unwrap();
                p
            })
            .collect();
        let items: Vec<_> = paths
            .iter()
            .enumerate()
            .map(|(i, p)| Resolved {
                number: i + 1,
                name: format!("{i}.png"),
                path: p.clone(),
                mime: "image/png".into(),
            })
            .collect();
        let p = prepare(&items, &[], 2, u64::MAX);
        assert_eq!(p.inline.len(), 2);
        assert_eq!(p.inline[0]["source"]["media_type"], "image/png");
        assert!(p.list.contains("Images 1–2 are shown above"));
        assert_eq!(
            inline_bytes(&p.inline),
            16,
            "two 5-byte files are 8 base64 chars each"
        );
        assert!(prepare(&items, &[], 0, u64::MAX).inline.is_empty());
        // A spent session budget means paths only.
        let spent = prepare(&items, &[], 20, 10);
        assert_eq!(spent.inline.len(), 1);
        assert!(prepare(&items, &[], 20, 0).inline.is_empty());
    }

    #[test]
    fn claude_line_without_images_is_unchanged_and_with_images_puts_text_last() {
        // Exactly the expression `send_message_from` used before images existed.
        let before = serde_json::json!({
            "type": "user",
            "message": { "role": "user", "content": "hi" }
        })
        .to_string();
        assert_eq!(claude_user_line("hi".into(), vec![]), before);
        let img = serde_json::json!({ "type": "image", "source": { "type": "base64", "media_type": "image/png", "data": "AA" } });
        let line = claude_user_line("look".into(), vec![img.clone()]);
        let v: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(v["message"]["content"][0], img);
        assert_eq!(
            v["message"]["content"][1],
            serde_json::json!({ "type": "text", "text": "look" })
        );
        // What gets stored is the text alone.
        let stored: serde_json::Value = serde_json::from_str(&persisted_line(&line)).unwrap();
        assert_eq!(stored["message"]["content"], "look");
    }

    #[test]
    fn persisted_line_drops_images_and_keeps_text_as_a_string() {
        let line = serde_json::json!({
            "type": "user",
            "message": { "role": "user", "content": [
                { "type": "image", "source": { "type": "base64", "media_type": "image/png", "data": "AAAA" } },
                { "type": "text", "text": "hello\n\n<attached_images>\n1. a — /p\n</attached_images>" }
            ]}
        })
        .to_string();
        let stored = persisted_line(&line);
        let v: serde_json::Value = serde_json::from_str(&stored).unwrap();
        assert_eq!(
            v["message"]["content"],
            "hello\n\n<attached_images>\n1. a — /p\n</attached_images>"
        );
        assert!(!stored.contains("AAAA"));
        // Plain string content is untouched, byte for byte.
        let plain = r#"{"type":"user","message":{"role":"user","content":"hi"}}"#;
        assert_eq!(persisted_line(plain), plain);
    }
}

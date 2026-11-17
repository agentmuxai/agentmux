// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
//! What the window that asks to send a draft says about the thread the draft replies into: which
//! thread it is, who is in it, and a warning when none of the draft's recipients has taken part in
//! it (a draft for one person that sits in another person's thread, #47852). It is shown text only:
//! the digest of the draft, which decides what is sent, does not take it. The pure half, as
//! [`super::gmail_write`] is: the flow that fetches the thread is [`super::gmail_flow`].

use serde_json::Value;

use super::gmail::{get, header, id_arg, line, url_of};
use super::gmail_write::DraftView;
use super::{clip, GoogleRequest};

/// The warning, as the window shows it and as the refusal repeats it.
pub const NO_PARTICIPANT: &str = "None of these recipients has taken part in that thread.";
const UNCHECKED: &str = "This draft replies into a thread, but the thread could not be read, so its recipients were not checked against it.";
/// How many of a thread's addresses the window lists, and how many it keeps to compare with.
const SHOWN_PEOPLE: usize = 5;
const KEPT_PEOPLE: usize = 500;

/// The request for the headers that say who is in a thread and what it is called.
pub fn context_plan(thread_id: &str) -> Result<GoogleRequest, String> {
    let id = id_arg("thread_id", Some(thread_id))?;
    let mut q = vec![("format", "metadata".to_string())];
    for h in ["From", "To", "Cc", "Bcc", "Subject"] {
        q.push(("metadataHeaders", h.to_string()));
    }
    Ok(get(url_of(&["threads", &id])?, q))
}

/// A thread as far as the window needs it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadContext {
    pub subject: String,
    pub messages: usize,
    /// Every address that wrote or received a message in it (lower case, first seen first).
    pub people: Vec<String>,
}

/// From Google's `threads.get` answer (format metadata).
pub fn context_of(thread: &Value) -> ThreadContext {
    let all = thread.get("messages").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
    let mut people: Vec<String> = Vec::new();
    for m in all {
        let p = m.get("payload").unwrap_or(&Value::Null);
        for name in ["From", "To", "Cc", "Bcc"] {
            for a in addresses_in(header(p, name)) {
                if people.len() < KEPT_PEOPLE && !people.contains(&a) {
                    people.push(a);
                }
            }
        }
    }
    let subject = all.first().map(|m| line(header(m.get("payload").unwrap_or(&Value::Null), "Subject"), 200)).unwrap_or_default();
    ThreadContext { subject, messages: all.len(), people }
}

/// The addresses in a header's value (`To`, `Cc`, `From`), lower case. A name is never read as an
/// address: with an `<address>` in the entry, only that counts, so a name that spells another
/// person's address can't put them in the thread.
pub fn addresses_in(value: &str) -> Vec<String> {
    let (mut out, mut entry) = (Vec::new(), String::new());
    let (mut quoted, mut escaped, mut angle) = (false, false, false);
    for c in value.chars().chain(std::iter::once(',')) {
        match c {
            _ if escaped => escaped = false,
            '\\' if quoted => escaped = true,
            '"' => quoted = !quoted,
            '<' if !quoted => angle = true,
            '>' if !quoted => angle = false,
            ',' | ';' if !quoted && !angle => {
                out.extend(entry_address(&entry));
                entry.clear();
                continue;
            }
            _ => {}
        }
        entry.push(c);
    }
    out
}

fn entry_address(entry: &str) -> Option<String> {
    let inner = match (entry.rfind('<'), entry.rfind('>')) {
        (Some(open), Some(close)) if open < close => &entry[open + 1..close],
        _ => entry,
    };
    let a = inner.trim().trim_matches('"').to_ascii_lowercase();
    let (local, domain) = a.split_once('@')?;
    (!local.is_empty() && !domain.is_empty() && !domain.contains('@') && !a.chars().any(|c| c.is_whitespace() || c.is_control())).then_some(a)
}

/// What the window adds for the thread a draft replies into.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ThreadNote {
    /// Shown first: no recipient is in the thread, or the thread could not be read.
    pub warning: Option<String>,
    /// Which thread, and who is in it.
    pub line: Option<String>,
    /// Whether no recipient has taken part in the thread: the refusal says so too.
    pub strangers: bool,
}

impl ThreadNote {
    /// The draft's recipients against the thread `ctx`; `own` is the account's address, left out of
    /// the list of who is in the thread (it is in nearly every thread) but not of the comparison.
    pub fn of(view: &DraftView, ctx: &ThreadContext, own: &str) -> ThreadNote {
        let recipients: Vec<String> = [&view.to, &view.cc, &view.bcc].iter().flat_map(|r| addresses_in(r)).collect();
        let strangers = !recipients.iter().any(|r| ctx.people.contains(r));
        let own = own.trim().to_ascii_lowercase();
        let others: Vec<&String> = ctx.people.iter().filter(|p| **p != own).collect();
        let who = if others.is_empty() {
            String::new()
        } else {
            let shown: Vec<&str> = others.iter().take(SHOWN_PEOPLE).map(|p| p.as_str()).collect();
            let more = if others.len() > SHOWN_PEOPLE { format!(" and {} more", others.len() - SHOWN_PEOPLE) } else { String::new() };
            format!(", with {}{more}", shown.join(", "))
        };
        let subject = if ctx.subject.is_empty() { "(no subject)" } else { &ctx.subject };
        let line = format!("Replies in the thread: {} ({} message{}{who})", clip(subject, 120), ctx.messages, if ctx.messages == 1 { "" } else { "s" });
        ThreadNote { warning: strangers.then(|| NO_PARTICIPANT.to_string()), line: Some(line), strangers }
    }

    /// The thread could not be read: the person is told, and decides.
    pub fn unchecked() -> ThreadNote {
        ThreadNote { warning: Some(UNCHECKED.to_string()), line: None, strangers: false }
    }
}

#[cfg(test)]
#[path = "gmail_thread_tests.rs"]
mod tests;

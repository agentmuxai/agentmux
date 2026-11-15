// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The running summary (`continuity_state.rs`) as sections, so a summary that is
//! too long is shortened by what it can spare, never by where it happens to end
//! (SPEC_AGENT_PREP_BEFORE_SESSION_RESTART_2026_11_14.md §6).
//!
//! The summary is a list of `## ` sections. A plain cut at a character count
//! lands in whichever section holds that character: with long "Done" lists that
//! is "In flight", and the identifiers at the end are lost first. Here the
//! sections that say where the work stands are never cut, and the others lose
//! their oldest items first, with a count of what went.

/// Only these are ever shortened, in this order, oldest items first. Every other
/// section is kept whole: the person's goal and last request, what is in flight,
/// what waits on them, the identifiers and their preferences.
const SHRINKABLE: [&str; 3] = ["done (unverified)", "done (verified)", "decisions and why"];
/// The order the packet prints the sections in, whatever order the summarizer
/// wrote them: where the work stands first, the finished work last.
const PACKET_ORDER: [&str; 9] = [
    "current goal",
    "last user request",
    "in flight",
    "waiting on the user",
    "exact identifiers",
    "decisions and why",
    "user preferences",
    "done (unverified)",
    "done (verified)",
];

struct Section {
    /// The `## ` line, without its newline; empty for text before the first heading.
    heading: String,
    /// The lines under it.
    body: Vec<String>,
}

impl Section {
    fn name(&self) -> String {
        self.heading.trim_start_matches('#').trim().to_ascii_lowercase()
    }
    fn is(&self, name: &str) -> bool {
        self.name().starts_with(name)
    }
    fn chars(&self) -> usize {
        self.render().chars().count()
    }
    fn render(&self) -> String {
        let mut out = String::new();
        if !self.heading.is_empty() {
            out.push_str(&self.heading);
            out.push('\n');
        }
        for line in &self.body {
            out.push_str(line);
            out.push('\n');
        }
        out
    }
}

fn parse(text: &str) -> Vec<Section> {
    let mut sections: Vec<Section> = Vec::new();
    for line in text.lines() {
        if line.starts_with("## ") {
            sections.push(Section { heading: line.to_string(), body: Vec::new() });
        } else {
            if sections.is_empty() {
                sections.push(Section { heading: String::new(), body: Vec::new() });
            }
            sections.last_mut().expect("a section was just pushed").body.push(line.to_string());
        }
    }
    sections
}

fn render(sections: &[Section]) -> String {
    sections.iter().map(Section::render).collect::<String>().trim_end().to_string()
}

/// The notice a shortened section carries on its first line.
fn omitted_line(n: usize) -> String {
    format!("({n} older items omitted)")
}

/// `n` from a notice line written by [`omitted_line`].
fn omitted_count(line: &str) -> Option<usize> {
    line.trim().strip_prefix('(')?.strip_suffix(" older items omitted)")?.parse().ok()
}

/// Whether `line` opens a list item, as opposed to continuing the one before it.
fn starts_item(line: &str) -> bool {
    let t = line.trim_start();
    if line.starts_with([' ', '\t']) {
        return false;
    }
    t.starts_with("- ")
        || t.starts_with("* ")
        || t.starts_with("+ ")
        || t.split_once(". ").is_some_and(|(n, _)| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()))
}

/// `body` as the lines before its first item, then one entry per item (its
/// wrapped lines with it).
fn items(body: &[String]) -> (Vec<String>, Vec<Vec<String>>) {
    let mut lead: Vec<String> = Vec::new();
    let mut items: Vec<Vec<String>> = Vec::new();
    for line in body {
        if starts_item(line) {
            items.push(vec![line.clone()]);
        } else if let Some(last) = items.last_mut() {
            last.push(line.clone());
        } else {
            lead.push(line.clone());
        }
    }
    (lead, items)
}

/// Drop the oldest items of `section` until the whole text is at most `target`
/// characters, or it has none left; returns how many went (counting a notice it
/// already carried).
fn shorten_section(section: &mut Section, text_chars: usize, target: usize) -> usize {
    let (mut lead, mut items) = items(&section.body);
    let earlier = lead.first().and_then(|l| omitted_count(l)).unwrap_or(0);
    if earlier > 0 {
        lead.remove(0);
    }
    let mut over = text_chars.saturating_sub(target);
    let mut dropped = 0;
    // A notice costs a line: allow for it up front.
    over += omitted_line(earlier + items.len()).chars().count() + 1;
    while over > 0 && !items.is_empty() {
        let gone = items.remove(0);
        over = over.saturating_sub(gone.iter().map(|l| l.chars().count() + 1).sum());
        dropped += 1;
    }
    let total = earlier + dropped;
    section.body = Vec::new();
    if total > 0 {
        section.body.push(omitted_line(total));
    }
    section.body.extend(lead);
    section.body.extend(items.into_iter().flatten());
    dropped
}

/// `text` shortened to at most `cap` characters when it can be, and how many
/// list items that took out. Whole sections are never cut mid-line: the
/// protected ones are kept whole, so when they alone are over `cap` the result
/// is longer than `cap` rather than cut.
pub(crate) fn shorten(text: &str, cap: usize) -> (String, usize) {
    if text.chars().count() <= cap {
        return (text.to_string(), 0);
    }
    let mut sections = parse(text);
    let mut dropped = 0;
    for name in SHRINKABLE {
        let total: usize = sections.iter().map(Section::chars).sum();
        if total <= cap {
            break;
        }
        for section in sections.iter_mut().filter(|s| s.is(name)) {
            dropped += shorten_section(section, total, cap);
        }
    }
    (render(&sections), dropped)
}

/// `text` with its sections in the packet's order (see [`PACKET_ORDER`]): the
/// ones it names first, any other after them in the order they were written, a
/// text with no sections as it is.
pub(crate) fn in_packet_order(text: &str) -> String {
    let sections = parse(text);
    if sections.iter().all(|s| s.heading.is_empty()) {
        return text.to_string();
    }
    let rank = |s: &Section| PACKET_ORDER.iter().position(|n| s.is(n)).unwrap_or(PACKET_ORDER.len());
    let mut ordered: Vec<&Section> = sections.iter().collect();
    // Text before the first heading stays first; the sort is stable.
    ordered.sort_by_key(|s| if s.heading.is_empty() { 0 } else { rank(s) + 1 });
    ordered.into_iter().map(Section::render).collect::<String>().trim_end().to_string()
}

#[cfg(test)]
#[path = "continuity_sections_tests.rs"]
mod tests;

// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Personal Memory as an index: one line per file, so the agent knows what it has and opens a file when it
//! applies (docs/specs/SPEC_MEMORY_DELIVERY_INDEX_NOT_FULL_TEXT_2026_11_17.md §3.2, §3.3).
//!
//! The index is complete by construction: every file appears once, in the index or, when marked `deliver:
//! always` and within the limit, in full. A long index shortens its descriptions, then drops them; it never
//! drops a file. It goes first in a delivery and stays under [`INDEX_BUDGET_CHARS`], so it sits whole in
//! the first part whatever order the parts arrive in.

use super::delivery::Entry;
use super::fit::frontmatter_value;

/// The most characters the index takes.
pub const INDEX_BUDGET_CHARS: usize = 8_000;
/// The most files marked `deliver: always` that go in full.
pub const ALWAYS_MAX_FILES: usize = 5;
/// The most characters of those files, together.
pub const ALWAYS_MAX_CHARS: usize = 6_000;
/// A description's longest form, then its shortened form once the index is over budget.
const DESCRIPTION_CHARS: [usize; 3] = [110, 60, 0];

/// Whether a file's frontmatter marks it to be sent in full every time (`deliver: always`, at any indent).
pub fn is_always(text: &str) -> bool {
    frontmatter_value(text, "deliver").is_some_and(|v| v.eq_ignore_ascii_case("always"))
}

/// What the index lists and how it tells the agent to read a file.
pub struct IndexPlan<'a> {
    /// Every Personal file that is not delivered in full.
    pub files: &'a [Entry],
    /// Names of files marked always that went over the limit, so their line says why.
    pub over_limit: &'a [String],
    /// Files delivered in full because they are marked always.
    pub always: usize,
    /// A container agent can't open the host's folder: its lines give the `Memory` tool's form only.
    pub tool_only: bool,
}

impl IndexPlan<'_> {
    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }
}

fn description_of(e: &Entry) -> String {
    if let Some(d) = frontmatter_value(&e.text, "description") {
        return d.split_whitespace().collect::<Vec<_>>().join(" ");
    }
    let body = e.text.split_once("\n---").filter(|_| e.text.starts_with("---")).map_or(e.text.as_str(), |(_, rest)| rest);
    body.lines().map(str::trim).find(|l| !l.is_empty() && !l.starts_with('#') && *l != "---").unwrap_or("").to_string()
}

fn line(e: &Entry, over_limit: bool, desc_chars: usize) -> String {
    let kind = frontmatter_value(&e.text, "type").unwrap_or_else(|| "unlisted".to_string());
    let stem = e.name.strip_suffix(".md").unwrap_or(&e.name);
    let title = frontmatter_value(&e.text, "name").filter(|n| n != stem && *n != e.name);
    let mut out = format!("- {}", e.name);
    if let Some(t) = title {
        out.push_str(&format!(" · {t}"));
    }
    out.push_str(&format!(" ({kind})"));
    let mut desc: String = description_of(e).chars().take(desc_chars).collect();
    if !desc.is_empty() {
        if description_of(e).chars().count() > desc_chars {
            desc = desc.trim_end().to_string() + "…";
        }
        out.push_str(&format!(" — {desc}"));
    }
    if over_limit {
        out.push_str(&format!(
            " [marked always, but over the limit of {ALWAYS_MAX_FILES} files and {ALWAYS_MAX_CHARS} characters; read it]"
        ));
    }
    out
}

/// The index's lines at the longest description that fits [`INDEX_BUDGET_CHARS`], as `(entry name, line)`.
pub fn lines(plan: &IndexPlan) -> Vec<(String, String)> {
    for chars in DESCRIPTION_CHARS {
        let lines: Vec<(String, String)> =
            plan.files.iter().map(|e| (e.name.clone(), line(e, plan.over_limit.contains(&e.name), chars))).collect();
        let total: usize = lines.iter().map(|(_, l)| l.chars().count() + 1).sum();
        if total <= INDEX_BUDGET_CHARS || chars == 0 {
            return lines;
        }
    }
    Vec::new()
}

/// The folder the index files live in, when they share one.
fn folder_of(files: &[Entry]) -> Option<String> {
    let first = files.first()?.path.as_deref()?;
    let dir = std::path::Path::new(first).parent()?.to_string_lossy().replace('\\', "/");
    files
        .iter()
        .all(|e| e.path.as_deref().and_then(|p| std::path::Path::new(p).parent()).is_some_and(|d| d.to_string_lossy().replace('\\', "/") == dir))
        .then_some(dir)
}

/// The index section: heading, the instruction to read before acting, then a line per file. `None` when no
/// file is listed.
pub fn render(plan: &IndexPlan) -> Option<String> {
    if plan.is_empty() {
        return None;
    }
    let n = plan.files.len();
    let noun = if n == 1 { "file" } else { "files" };
    let mut out = format!("# Personal Memory ({n} {noun}, listed as an index)\n");
    out.push_str("Read a file before you act on what its line describes: the line is all you have until you do.\n");
    let folder = if plan.tool_only { None } else { folder_of(plan.files) };
    match folder {
        Some(dir) => out.push_str(&format!(
            "Read one with the Memory tool (action read, scope personal, filename <file>) or open it in {dir}.\n"
        )),
        None => out.push_str("Read one with the Memory tool (action read, scope personal, filename <file>).\n"),
    }
    if plan.always > 0 {
        let noun = if plan.always == 1 { "file is" } else { "files are" };
        out.push_str(&format!("{} more {noun} marked always and follow in full.\n", plan.always));
    }
    for (_, l) in lines(plan) {
        out.push_str(&l);
        out.push('\n');
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::memory::delivery::Tier;

    fn file(name: &str, text: &str) -> Entry {
        Entry { label: name.into(), tier: Tier::Personal, text: text.into(), name: name.into(), system: false, bundle_id: None, path: Some(format!("/mem/{name}")) }
    }

    fn doc(name: &str, kind: &str, desc: &str) -> String {
        format!("---\nname: {name}\ndescription: {desc}\nmetadata:\n  type: {kind}\n---\n\nbody of {name}\n")
    }

    #[test]
    fn a_line_has_the_file_the_type_and_the_description() {
        let f = file("rule-one.md", &doc("rule-one", "feedback", "When a build slot is busy, wait for it"));
        let plan = IndexPlan { files: &[f], over_limit: &[], always: 0, tool_only: false };
        let text = render(&plan).unwrap();
        assert!(text.contains("- rule-one.md (feedback) — When a build slot is busy, wait for it"), "{text}");
        assert!(text.contains("open it in /mem."), "{text}");
        assert!(!text.contains("body of rule-one"), "the body is not in the index");
    }

    #[test]
    fn a_container_is_given_the_tool_form_only() {
        let f = file("a.md", &doc("a", "user", "d"));
        let text = render(&IndexPlan { files: &[f], over_limit: &[], always: 0, tool_only: true }).unwrap();
        assert!(!text.contains("/mem"), "{text}");
        assert!(text.contains("Memory tool"));
    }

    #[test]
    fn a_file_without_frontmatter_is_listed_by_its_first_line() {
        let f = file("loose.md", "# Heading\n\nthe first real line\nmore\n");
        let text = render(&IndexPlan { files: &[f], over_limit: &[], always: 0, tool_only: false }).unwrap();
        assert!(text.contains("- loose.md (unlisted) — the first real line"), "{text}");
    }

    #[test]
    fn the_index_is_complete_and_inside_its_budget_for_any_number_of_files() {
        for n in [0usize, 1, 9, 40, 120, 400] {
            let files: Vec<Entry> = (0..n).map(|i| file(&format!("memory-file-number-{i:03}.md"), &doc(&format!("m{i}"), "project", &"long description words ".repeat(20)))).collect();
            let plan = IndexPlan { files: &files, over_limit: &[], always: 0, tool_only: false };
            let Some(text) = render(&plan) else {
                assert_eq!(n, 0);
                continue;
            };
            for f in &files {
                assert_eq!(text.matches(&format!("- {} ", f.name)).count(), 1, "{} listed once at n={n}", f.name);
            }
            let body: usize = lines(&plan).iter().map(|(_, l)| l.chars().count() + 1).sum();
            if n <= 120 {
                assert!(body <= INDEX_BUDGET_CHARS, "n={n}: {body}");
            }
        }
    }

    #[test]
    fn a_long_description_is_shortened_before_a_file_is_dropped() {
        let files: Vec<Entry> = (0..60).map(|i| file(&format!("f{i:02}.md"), &doc("x", "feedback", &"word ".repeat(40)))).collect();
        let plan = IndexPlan { files: &files, over_limit: &[], always: 0, tool_only: false };
        assert_eq!(lines(&plan).len(), 60);
        assert!(lines(&plan).iter().all(|(_, l)| l.chars().count() < 160));
    }

    #[test]
    fn an_over_limit_always_file_says_why() {
        let f = file("big.md", &doc("big", "feedback", "d"));
        let text = render(&IndexPlan { files: &[f], over_limit: &["big.md".to_string()], always: 2, tool_only: false }).unwrap();
        assert!(text.contains("marked always, but over the limit"), "{text}");
        assert!(text.contains("2 more files are marked always"), "{text}");
    }

    #[test]
    fn deliver_always_is_found_at_any_indent_and_only_as_always() {
        assert!(is_always("---\nname: a\nmetadata:\n  deliver: always\n---\nx"));
        assert!(is_always("---\ndeliver: Always\n---\nx"));
        assert!(!is_always("---\ndeliver: never\n---\nx"));
        assert!(!is_always("no frontmatter, deliver: always"));
    }
}

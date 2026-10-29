// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The push scan: what an agent's `git push` is refused for.
//!
//! A credential is what a person's agent is most likely to leak, and no repository, public or
//! private, is a place for one. The scan looks at the lines a push adds (never at what it removes
//! or leaves alone) for the formats whose shape is certain, and says where and what kind, never the
//! text itself: the refusal would otherwise put the credential in the agent's transcript.

/// One credential-shaped string a push would add.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub file: String,
    pub line: u32,
    pub category: &'static str,
}

/// The most findings reported; a push with more is refused all the same.
const MAX_FINDINGS: usize = 20;

fn is_alnum(c: char) -> bool {
    c.is_ascii_alphanumeric()
}

fn is_key_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-'
}

/// Whether `text` holds `prefix` followed by at least `min` characters of `class`, the prefix not
/// being the tail of a longer word (`risk-...` is not a key).
fn has_run(text: &str, prefix: &str, min: usize, class: fn(char) -> bool) -> bool {
    let mut from = 0;
    while let Some(at) = text[from..].find(prefix) {
        let start = from + at;
        let after = start + prefix.len();
        let word_tail = text[..start].chars().next_back().is_some_and(is_alnum);
        if !word_tail && text[after..].chars().take_while(|c| class(*c)).count() >= min {
            return true;
        }
        from = start + prefix.len().max(1);
    }
    false
}

/// A category if `text` holds a credential of a format whose shape is certain.
pub fn credential_in(text: &str) -> Option<&'static str> {
    // The markers are built from pieces, so this file doesn't hold what it looks for.
    if text.contains(&["-----BE", "GIN "].concat()) && text.contains(&["PRIVATE", " KEY-----"].concat()) {
        return Some("private key");
    }
    if ["ghp_", "gho_", "ghu_", "ghs_", "ghr_"].iter().any(|p| has_run(text, p, 36, is_alnum)) || has_run(text, "github_pat_", 22, is_key_char) {
        return Some("GitHub token");
    }
    // Access key ids and temporary ones; the other `A???` prefixes (AGPA, AIDA, AROA, ANPA, ANVA)
    // are IAM group, user, role and policy ids, which are not credentials.
    if ["AKIA", "ASIA"].iter().any(|p| has_run(text, p, 16, |c| c.is_ascii_uppercase() || c.is_ascii_digit())) {
        return Some("AWS access key id");
    }
    if has_run(text, "sk-ant-", 20, is_key_char) {
        return Some("Anthropic API key");
    }
    if has_run(text, "sk-", 40, is_key_char) {
        return Some("OpenAI API key");
    }
    if ["xoxb-", "xoxp-", "xoxa-", "xoxr-", "xoxs-"].iter().any(|p| has_run(text, p, 20, |c| c.is_ascii_alphanumeric() || c == '-')) {
        return Some("Slack token");
    }
    if has_run(text, "AIza", 35, is_key_char) {
        return Some("Google API key");
    }
    if has_run(text, "sk_live_", 20, is_alnum) {
        return Some("Stripe live key");
    }
    None
}

/// The credentials in the lines a unified diff adds (`git log -p -U0`, `git diff -U0`).
pub fn scan_diff(diff: &str) -> Vec<Finding> {
    let mut found = Vec::new();
    let mut file = String::new();
    let mut line_no: u32 = 0;
    // Lines the current hunk still has: a line that begins `+++ ` inside one is an added line
    // (its text begins `++ `), not the next file's header.
    let (mut old_left, mut new_left) = (0u32, 0u32);
    // A merge commit is shown as a combined diff (`git log --cc`): one column per parent in front of
    // each line, and only what differs from every parent is in it. `cols` is that count in the
    // current hunk, 0 for a plain one.
    let mut cols = 0usize;
    for line in diff.lines() {
        let in_hunk = old_left > 0 || new_left > 0;
        let in_hunk = if line.starts_with("diff ") {
            // A new file's diff: nothing of the last hunk is left.
            (cols, old_left, new_left) = (0, 0, 0);
            false
        } else {
            in_hunk
        };
        if !in_hunk {
            if let Some(path) = line.strip_prefix("+++ ") {
                file = path.strip_prefix("b/").unwrap_or(path).trim_matches('"').to_string();
                continue;
            }
        }
        if line.starts_with("@@@") {
            cols = line.chars().take_while(|c| *c == '@').count() - 1;
            let (start, count) = combined_new_range(line);
            (old_left, new_left, line_no) = (0, count, start);
        } else if cols > 0
            && line.get(..cols).is_some_and(|p| p.chars().all(|c| matches!(c, ' ' | '+' | '-')) && (new_left > 0 || p.contains('-')))
        {
            let (prefix, text) = line.split_at(cols);
            if prefix.contains('-') {
                // Gone from the result.
            } else if prefix.chars().all(|c| c == '+') {
                // Added against every parent: what this merge brings that no parent had.
                if let Some(category) = credential_in(text) {
                    found.push(Finding { file: file.clone(), line: line_no, category });
                    if found.len() >= MAX_FINDINGS {
                        return found;
                    }
                }
                line_no += 1;
                new_left = new_left.saturating_sub(1);
            } else {
                // Already in a parent, so already wherever that parent is.
                line_no += 1;
                new_left = new_left.saturating_sub(1);
            }
        } else if let Some(hunk) = line.strip_prefix("@@") {
            cols = 0;
            // `@@ -old[,count] +new[,count] @@`
            let (old, new) = hunk_counts(hunk);
            (old_left, new_left) = (old.1, new.1);
            line_no = new.0;
        } else if let Some(added) = line.strip_prefix('+') {
            if let Some(category) = credential_in(added) {
                found.push(Finding { file: file.clone(), line: line_no, category });
                if found.len() >= MAX_FINDINGS {
                    return found;
                }
            }
            line_no += 1;
            new_left = new_left.saturating_sub(1);
        } else if line.starts_with('-') {
            old_left = old_left.saturating_sub(1);
        } else if line.starts_with(' ') {
            line_no += 1;
            old_left = old_left.saturating_sub(1);
            new_left = new_left.saturating_sub(1);
        }
    }
    found
}

/// `(start, count)` of the result side of a combined hunk header, `@@@ -a,b -c,d +e,f @@@`; a
/// missing count is 1.
fn combined_new_range(header: &str) -> (u32, u32) {
    let Some((_, after)) = header.rsplit_once(" +") else { return (0, 0) };
    let mut n = after.split(' ').next().unwrap_or("").split(',');
    let start = n.next().and_then(|v| v.parse().ok()).unwrap_or(0);
    let count = n.next().and_then(|v| v.parse().ok()).unwrap_or(1);
    (start, count)
}

/// `(start, count)` of the old and the new side of a hunk header's `-a[,b] +c[,d] @@`; a missing
/// count is 1.
fn hunk_counts(header: &str) -> ((u32, u32), (u32, u32)) {
    let side = |part: &str| {
        let mut n = part.trim_start_matches(['-', '+', ' ']).split(',');
        let start = n.next().and_then(|v| v.trim().parse().ok()).unwrap_or(0);
        let count = n.next().and_then(|v| v.trim().parse().ok()).unwrap_or(1);
        (start, count)
    };
    let ranges = header.split("@@").next().unwrap_or(header).trim();
    let mut parts = ranges.split(" +");
    let old = side(parts.next().unwrap_or(""));
    let new = side(parts.next().unwrap_or(""));
    (old, new)
}

/// What a refusal says: where and what, never the text.
pub fn refusal_text(findings: &[Finding]) -> String {
    let mut out = String::from("agentmux: this push is refused: it adds what looks like a credential.\n");
    for f in findings {
        out.push_str(&format!("  {}:{}  {}\n", f.file, f.line, f.category));
    }
    out.push_str("The text is not shown here. Take it out of the commits (and rotate it if it was real), then push again.\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    // Built from pieces, so this file holds no credential-shaped string of its own.
    fn gh() -> String {
        format!("{}{}", "ghp_", "a1B2c3D4e5F6g7H8i9J0k1L2m3N4o5P6q7R8")
    }
    fn aws() -> String {
        format!("{}{}", "AKIA", "ABCDEFGHIJKLMNOP")
    }
    fn anthropic() -> String {
        format!("{}{}", "sk-ant-", "api03-abcdefghijklmnopqrstuvwxyz0123456789")
    }

    #[test]
    fn the_formats_with_a_certain_shape_are_found_and_named() {
        assert_eq!(credential_in(&format!("token = \"{}\"", gh())), Some("GitHub token"));
        assert_eq!(credential_in(&format!("AWS_KEY={}", aws())), Some("AWS access key id"));
        assert_eq!(credential_in(&format!("k: {}", anthropic())), Some("Anthropic API key"));
        assert_eq!(credential_in(&["-----BE", "GIN RSA PRIV", "ATE KEY-----"].concat()), Some("private key"));
        assert_eq!(credential_in(&format!("{}{}", "github_pat_", "11ABCDEFG0abcdefghijkl_mnopqrstuvwxyz")), Some("GitHub token"));
        assert_eq!(credential_in(&format!("{}{}", "xoxb-", "1234567890-abcdefghijklmnop")), Some("Slack token"));
    }

    #[test]
    fn ordinary_text_and_short_lookalikes_are_not() {
        for text in [
            "let key = format!(\"ghp_{}\", n);",
            "AKIA is the prefix of an access key id",
            "ask-for-the-thing-that-you-want-to-have-no-longer-than-this-one",
            "risk-averse-and-long-enough-to-look-like-a-key-0123456789abcdef",
            "sk-short",
            "// -----BEGIN is where it starts",
        ] {
            assert_eq!(credential_in(text), None, "{text}");
        }
        // An IAM role id is an identifier, not a credential.
        assert_eq!(credential_in(&["\"RoleId\": \"", "AROA", "KSCDLFT9R5DQP782U\""].concat()), None);
    }

    #[test]
    fn a_diff_names_the_file_and_the_line_of_each_added_credential() {
        let diff = format!(
            "diff --git a/src/a.rs b/src/a.rs\n--- a/src/a.rs\n+++ b/src/a.rs\n@@ -3,0 +4,3 @@\n+fn ok() {{}}\n+let k = \"{}\";\n+let n = 1;\n\
             diff --git a/b.env b/b.env\n--- /dev/null\n+++ b/b.env\n@@ -0,0 +1,2 @@\n+A=1\n+KEY={}\n-OLD={}\n",
            gh(),
            aws(),
            anthropic()
        );
        assert_eq!(
            scan_diff(&diff),
            vec![
                Finding { file: "src/a.rs".into(), line: 5, category: "GitHub token" },
                Finding { file: "b.env".into(), line: 2, category: "AWS access key id" },
            ],
            "a removed line is not a finding"
        );
    }

    #[test]
    fn an_added_line_that_looks_like_a_file_header_is_still_scanned() {
        // The added line `++ <key>` shows in the diff as `+++ <key>`.
        let diff = format!(
            "--- a/x\n+++ b/x\n@@ -0,0 +1,2 @@\n+first\n+++ {}\n@@ -5,1 +5,0 @@\n-gone\n+++ b/next\n@@ -0,0 +1,1 @@\n+{}\n",
            gh(),
            aws()
        );
        assert_eq!(
            scan_diff(&diff),
            vec![
                Finding { file: "x".into(), line: 2, category: "GitHub token" },
                Finding { file: "next".into(), line: 1, category: "AWS access key id" },
            ]
        );
        assert_eq!(hunk_counts(" -3,2 +4 @@ fn x"), ((3, 2), (4, 1)));
    }

    #[test]
    fn a_merge_is_scanned_for_what_no_parent_had() {
        // `git log -p --cc`: the first line is in the result only through the merge, the second is
        // already in the second parent, and the third is a plain commit after it.
        let diff = format!(
            "diff --cc a.txt\nindex 111,222..333\n--- a/a.txt\n+++ b/a.txt\n@@@ -1,2 -1,2 +1,3 @@@\n  base\n++key = {}\n +from_main = {}\n- gone\ndiff --git a/b b/b\n--- a/b\n+++ b/b\n@@ -0,0 +1 @@\n+{}\n",
            gh(),
            aws(),
            aws()
        );
        assert_eq!(
            scan_diff(&diff),
            vec![
                Finding { file: "a.txt".into(), line: 2, category: "GitHub token" },
                Finding { file: "b".into(), line: 1, category: "AWS access key id" },
            ]
        );
        assert_eq!(combined_new_range("@@@ -1,2 -1 +4,3 @@@ fn x"), (4, 3));
    }

    #[test]
    fn a_refusal_never_contains_the_matched_text() {
        let text = refusal_text(&[Finding { file: "x".into(), line: 1, category: "GitHub token" }]);
        assert!(text.contains("x:1  GitHub token"));
        assert!(!text.contains("ghp_"));
    }
}

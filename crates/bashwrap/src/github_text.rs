// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The text scan: what an agent's `gh` is refused for when the text it sends holds a credential.
//!
//! The push scan (`secret_scan.rs`) looks at the lines a push adds; this looks at the words an
//! agent puts in a pull request, an issue, a comment, a review, release notes or a `gh api` field,
//! with the same formats and the same rule: say where and what kind, never the text itself.
//! Spec: `SPEC_CONNECT_WITH_GITHUB` §5.6, phase G3b.
//!
//! A local check: a process that runs `gh` another way (or `command gh`) skips it, as it skips
//! every guard of the agent's shell (§9 item 8). It stops an agent's mistake, not a hostile process.
//!
//! The `gh` function the wrapper exports calls `github-text-guard -- <gh args>` before it runs
//! `gh`. The reply is the arguments to run, each followed by a NUL, then `ok`; or `refused` (the
//! reason is on stderr). A body given on stdin (`--body-file -`, `-F -`, `--input -`) is read here,
//! scanned, and handed to `gh` as a file, because stdin can't be read twice.

use crate::secret_scan::credential_in;
use std::io::{Read, Write};

/// The most findings reported; a command with more is refused all the same.
const MAX_HITS: usize = 20;
/// The largest text read for the scan (a PR body is at most 65,536 characters; a `gh api --input`
/// file can be bigger). A larger one is refused: it can't be scanned.
const MAX_TEXT_BYTES: u64 = 8 * 1024 * 1024;
/// A stdin body goes to `gh` as a file in this directory of the temp folder, and files older than a day are removed.
const STDIN_DIR: &str = "agentmux-gh-text";
const STDIN_KEEP: std::time::Duration = std::time::Duration::from_secs(24 * 60 * 60);

/// Where a credential-shaped string is in the command, and what kind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit {
    pub place: String,
    pub line: u32,
    pub category: &'static str,
}

/// The two command words of `gh <args>` (`pr create`), skipping flags and the value of `-R`/`--repo`.
fn command_words(args: &[String]) -> (Option<&str>, Option<&str>) {
    let mut words = Vec::new();
    let mut i = 0;
    while i < args.len() && words.len() < 2 {
        let a = args[i].as_str();
        if a == "-R" || a == "--repo" {
            i += 1;
        } else if !a.starts_with('-') {
            words.push(a);
        }
        i += 1;
    }
    (words.first().copied(), words.get(1).copied())
}

/// Does `gh <args>` send text to GitHub that someone wrote: a PR, issue, comment, review, merge
/// message, release notes or an API call?
pub fn sends_text(args: &[String]) -> bool {
    match command_words(args) {
        (Some("pr"), Some("create" | "edit" | "comment" | "review" | "merge" | "close" | "reopen")) => true,
        (Some("issue"), Some("create" | "edit" | "comment" | "close" | "reopen")) => true,
        (Some("release"), Some("create" | "edit")) => true,
        (Some("api"), _) => true,
        _ => false,
    }
}

fn is_api(args: &[String]) -> bool {
    command_words(args).0 == Some("api")
}

/// Flags whose value is a file of text: `--body-file`/`-F` (and `--notes-file`) on the commands
/// that take a body, `--input` on `gh api`. A file argument of `-` is stdin.
fn takes_file(flag: &str, api: bool) -> bool {
    match flag {
        "--body-file" | "--notes-file" => true,
        "-F" => !api,
        "--input" => api,
        _ => false,
    }
}

/// An argument that is a flag, not text: it starts with `-`, is short and has no whitespace. A body
/// that begins with `- ` (a list) or `---` is text and is scanned.
fn is_flag(a: &str) -> bool {
    a.len() > 1 && a.starts_with('-') && a.len() <= 40 && !a.chars().any(char::is_whitespace)
}

/// `--name` of a `--name=value` argument, if `arg` is one.
fn long_flag_value(arg: &str) -> Option<(&str, &str)> {
    let (flag, value) = arg.split_once('=')?;
    let name = flag.strip_prefix("--")?;
    (name.starts_with(|c: char| c.is_ascii_alphabetic()) && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')).then_some((flag, value))
}

/// One piece of text the command sends: given on the command line, or in a file (`-` for stdin).
#[derive(Debug, PartialEq, Eq)]
enum Source {
    Inline(String),
    File(String),
}

/// Everything in `args` that is text: each argument itself, and the files the file flags name,
/// each with the place to report (the flag before it, else the argument's position).
fn sources(args: &[String]) -> Vec<(String, Source)> {
    let api = is_api(args);
    let mut out = Vec::new();
    for (i, arg) in args.iter().enumerate() {
        let prev = i.checked_sub(1).map(|p| args[p].as_str()).filter(|p| is_flag(p));
        // `--flag=value`
        if let Some((flag, value)) = long_flag_value(arg) {
            if takes_file(flag, api) {
                out.push((flag.to_string(), Source::File(value.to_string())));
            } else if api && matches!(flag, "--field") {
                push_field(&mut out, flag, value);
            } else {
                out.push((flag.to_string(), Source::Inline(value.to_string())));
            }
            continue;
        }
        if is_flag(arg) {
            continue;
        }
        let place = prev.map(str::to_string).unwrap_or_else(|| format!("argument {}", i + 1));
        match prev {
            Some(flag) if takes_file(flag, api) => out.push((place, Source::File(arg.clone()))),
            Some("-F" | "--field") if api => push_field(&mut out, prev.unwrap_or("-F"), arg),
            Some("-f" | "--raw-field") if api => {
                let key = arg.split_once('=').map(|(k, _)| k).unwrap_or("");
                out.push((format!("{} {key}", prev.unwrap_or("-f")), Source::Inline(arg.clone())));
            }
            _ => out.push((place, Source::Inline(arg.clone()))),
        }
    }
    out
}

/// A `gh api` field `key=value` (`-F`/`--field`): `value` is text, or `@file` (`@-` for stdin).
fn push_field(out: &mut Vec<(String, Source)>, flag: &str, field: &str) {
    let (key, value) = field.split_once('=').unwrap_or(("", field));
    let place = format!("{flag} {key}");
    match value.strip_prefix('@') {
        Some(path) => out.push((place, Source::File(path.to_string()))),
        None => out.push((place, Source::Inline(value.to_string()))),
    }
}

/// The credentials in `text`, a line at a time.
fn scan_text(place: &str, text: &str, hits: &mut Vec<Hit>) {
    for (n, line) in text.lines().enumerate() {
        if hits.len() >= MAX_HITS {
            return;
        }
        if let Some(category) = credential_in(line) {
            let hit = Hit { place: place.to_string(), line: n as u32 + 1, category };
            if !hits.contains(&hit) {
                hits.push(hit);
            }
        }
    }
}

/// What the scan of a command found.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Scan {
    pub hits: Vec<Hit>,
    /// A file too large to scan: the command is refused.
    pub too_large: Vec<String>,
    /// The command reads a body from stdin.
    pub uses_stdin: bool,
}

/// Scan `gh <args>`. `read_file` gives a file's text: `Ok(None)` when it isn't there (gh will say
/// so itself), `Err` when it is too large to scan. `stdin` gives what stdin holds, read at most once.
pub fn scan(args: &[String], read_file: impl Fn(&str) -> Result<Option<String>, ()>, mut stdin: impl FnMut() -> Result<String, ()>) -> Scan {
    let mut result = Scan::default();
    for (place, source) in sources(args) {
        match source {
            Source::Inline(text) => scan_text(&place, &text, &mut result.hits),
            Source::File(path) if path == "-" => {
                result.uses_stdin = true;
                match stdin() {
                    Ok(text) => scan_text(&place, &text, &mut result.hits),
                    Err(()) => result.too_large.push(format!("{place} (stdin)")),
                }
            }
            Source::File(path) => match read_file(&path) {
                Ok(Some(text)) => scan_text(&format!("{place} {path}"), &text, &mut result.hits),
                Ok(None) => {}
                Err(()) => result.too_large.push(format!("{place} {path}")),
            },
        }
    }
    result
}

/// `args` with every stdin source (`-`, `@-`) replaced by `path`.
pub fn replace_stdin(args: &[String], path: &str) -> Vec<String> {
    let api = is_api(args);
    let mut out = Vec::with_capacity(args.len());
    for (i, arg) in args.iter().enumerate() {
        let prev = i.checked_sub(1).map(|p| args[p].as_str()).filter(|p| is_flag(p));
        let after_file_flag = prev.is_some_and(|p| takes_file(p, api));
        let after_field_flag = api && prev.is_some_and(|p| matches!(p, "-F" | "--field"));
        let replaced = if arg == "-" && after_file_flag {
            Some(path.to_string())
        } else if after_field_flag && arg.split_once('=').is_some_and(|(_, v)| v == "@-") {
            arg.split_once('=').map(|(k, _)| format!("{k}=@{path}"))
        } else if let Some((flag, value)) = long_flag_value(arg) {
            if value == "-" && takes_file(flag, api) {
                Some(format!("{flag}={path}"))
            } else if api && flag == "--field" && value.split_once('=').is_some_and(|(_, v)| v == "@-") {
                value.split_once('=').map(|(k, _)| format!("{flag}={k}=@{path}"))
            } else {
                None
            }
        } else {
            None
        };
        out.push(replaced.unwrap_or_else(|| arg.clone()));
    }
    out
}

/// The refusal: where, and what kind, never the text.
pub fn refusal_text(args: &[String], scan: &Scan) -> String {
    let (a, b) = command_words(args);
    let mut out = format!(
        "agentmux: `gh {} {}` is refused: the text it sends {}. Nothing was sent.\n",
        a.unwrap_or(""),
        b.unwrap_or(""),
        if scan.hits.is_empty() { "can't be checked" } else { "has what looks like a credential" }
    );
    for h in &scan.hits {
        out.push_str(&format!("  {}, line {}  {}\n", h.place, h.line, h.category));
    }
    for f in &scan.too_large {
        out.push_str(&format!("  {f}  too large to check (over {} MB)\n", MAX_TEXT_BYTES / (1024 * 1024)));
    }
    if scan.hits.is_empty() {
        out.push_str("Send less text, or put it in the repository instead.\n");
    } else {
        out.push_str("The text is not shown here. Take it out (and rotate it if it was real), then run the command again.\n");
    }
    out
}

fn read_file_limited(path: &str) -> Result<Option<String>, ()> {
    let Ok(meta) = std::fs::metadata(path) else { return Ok(None) };
    if !meta.is_file() {
        return Ok(None);
    }
    if meta.len() > MAX_TEXT_BYTES {
        return Err(());
    }
    Ok(std::fs::read(path).ok().map(|b| String::from_utf8_lossy(&b).into_owned()))
}

fn read_stdin_limited() -> Result<String, ()> {
    let mut buf = Vec::new();
    let _ = std::io::stdin().lock().take(MAX_TEXT_BYTES + 1).read_to_end(&mut buf);
    if buf.len() as u64 > MAX_TEXT_BYTES {
        return Err(());
    }
    Ok(String::from_utf8_lossy(&buf).into_owned())
}

/// Write `text` where `gh` can read it as a file, and remove the folder's files older than a day.
fn write_stdin_file(text: &str) -> Option<String> {
    let dir = std::env::temp_dir().join(STDIN_DIR);
    std::fs::create_dir_all(&dir).ok()?;
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for e in entries.flatten() {
            let old = e.metadata().and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).is_some_and(|age| age > STDIN_KEEP);
            if old {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let path = dir.join(format!("{}-{nanos}.txt", std::process::id()));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(&path).ok()?.write_all(text.as_bytes()).ok()?;
    Some(path.to_string_lossy().into_owned())
}

/// `github-text-guard -- <gh args>`: scan what the command sends. Prints the arguments to run
/// (a stdin body replaced by a file), each followed by a NUL, then `ok`; or `refused` with the
/// reason on stderr and exit status 1.
pub fn run_github_text_guard(gh_args: &[String]) -> i32 {
    let mut stdout = std::io::stdout();
    let mut finish = |args: &[String], verdict: &str| {
        for a in args {
            let _ = stdout.write_all(a.as_bytes());
            let _ = stdout.write_all(&[0]);
        }
        let _ = stdout.write_all(verdict.as_bytes());
        let _ = stdout.write_all(&[0]);
        let _ = stdout.flush();
    };
    if !sends_text(gh_args) {
        finish(gh_args, "ok");
        return 0;
    }
    let mut stdin_text = None;
    // Stdin is read once, whichever flags name it.
    let result = scan(gh_args, read_file_limited, || {
        if let Some(text) = &stdin_text {
            return Ok::<String, ()>(String::clone(text));
        }
        let text = read_stdin_limited()?;
        stdin_text = Some(text.clone());
        Ok(text)
    });
    if !result.hits.is_empty() || !result.too_large.is_empty() {
        eprint!("{}", refusal_text(gh_args, &result));
        finish(&[], "refused");
        return 1;
    }
    let args = match (result.uses_stdin, stdin_text) {
        (true, Some(text)) => match write_stdin_file(&text) {
            Some(path) => replace_stdin(gh_args, &path),
            None => {
                eprintln!("agentmux: this gh command is refused: its text was read from stdin and couldn't be handed on after the check.");
                finish(&[], "refused");
                return 1;
            }
        },
        _ => gh_args.to_vec(),
    };
    finish(&args, "ok");
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }
    // Built from pieces, so this file holds no credential-shaped string of its own.
    fn token() -> String {
        format!("{}{}", "ghp_", "a1B2c3D4e5F6g7H8i9J0k1L2m3N4o5P6q7R8")
    }
    fn no_files(_: &str) -> Result<Option<String>, ()> {
        Ok(None)
    }
    fn no_stdin() -> Result<String, ()> {
        Ok(String::new())
    }

    #[test]
    fn only_commands_that_send_text_are_scanned() {
        for yes in [
            &["pr", "create", "--title", "x"][..],
            &["pr", "comment", "12", "--body", "x"],
            &["issue", "create"],
            &["issue", "comment", "3"],
            &["pr", "review", "4", "--approve"],
            &["pr", "merge", "4", "--body", "x"],
            &["release", "create", "v1"],
            &["api", "repos/o/r/issues/1/comments"],
            &["pr", "-R", "o/r", "edit", "3"],
        ] {
            assert!(sends_text(&args(yes)), "{yes:?}");
        }
        for no in [&["pr", "list"][..], &["pr", "view", "3"], &["issue", "list"], &["repo", "view"], &["auth", "status"], &["--version"], &[]] {
            assert!(!sends_text(&args(no)), "{no:?}");
        }
    }

    #[test]
    fn a_credential_in_any_text_argument_is_found_with_its_flag_and_line() {
        let body = format!("first line\nthe key is {}\nlast", token());
        let s = scan(&args(&["pr", "create", "--title", "fine", "--body", &body]), no_files, no_stdin);
        assert_eq!(s.hits, vec![Hit { place: "--body".into(), line: 2, category: "GitHub token" }]);
        // The same in the `--flag=value` and short spellings.
        let s = scan(&args(&["issue", "comment", "3", &format!("--body={}", token())]), no_files, no_stdin);
        assert_eq!(s.hits, vec![Hit { place: "--body".into(), line: 1, category: "GitHub token" }]);
        let s = scan(&args(&["pr", "create", "-t", &token()]), no_files, no_stdin);
        assert_eq!(s.hits[0].place, "-t");
        // A body that begins with a dash is text, not a flag.
        let bullet = format!("- item
- key {}", token());
        assert_eq!(scan(&args(&["pr", "comment", "1", "--body", &bullet]), no_files, no_stdin).hits.len(), 1);
        let dashes = format!("--- notes
{}", token());
        assert_eq!(scan(&args(&["pr", "comment", "1", "-b", &dashes]), no_files, no_stdin).hits.len(), 1);
        // Clean text finds nothing.
        assert!(scan(&args(&["pr", "create", "--title", "Fix the thing", "--body", "It works."]), no_files, no_stdin).hits.is_empty());
    }

    #[test]
    fn a_body_file_is_read_and_a_missing_one_is_left_to_gh() {
        let read = |p: &str| -> Result<Option<String>, ()> { Ok((p == "body.md").then(|| format!("# Notes\n{}\n", token()))) };
        let s = scan(&args(&["pr", "create", "--body-file", "body.md"]), read, no_stdin);
        assert_eq!(s.hits, vec![Hit { place: "--body-file body.md".into(), line: 2, category: "GitHub token" }]);
        let s = scan(&args(&["pr", "comment", "1", "-F", "body.md"]), read, no_stdin);
        assert_eq!(s.hits.len(), 1);
        let s = scan(&args(&["pr", "create", "--body-file=body.md"]), read, no_stdin);
        assert_eq!(s.hits.len(), 1);
        let s = scan(&args(&["pr", "create", "--body-file", "nope.md"]), read, no_stdin);
        assert!(s.hits.is_empty() && s.too_large.is_empty());
        // Too large to scan is refused, not waved through.
        let big = |_: &str| -> Result<Option<String>, ()> { Err(()) };
        assert_eq!(scan(&args(&["pr", "create", "--body-file", "huge.md"]), big, no_stdin).too_large.len(), 1);
    }

    #[test]
    fn a_body_on_stdin_is_scanned_once_and_handed_on_as_a_file() {
        let a = args(&["pr", "create", "--title", "t", "--body-file", "-"]);
        let s = scan(&a, no_files, || Ok(format!("x\n{}\n", token())));
        assert!(s.uses_stdin);
        assert_eq!(s.hits[0].place, "--body-file");
        assert_eq!(replace_stdin(&a, "/tmp/b.txt"), args(&["pr", "create", "--title", "t", "--body-file", "/tmp/b.txt"]));
        assert_eq!(replace_stdin(&args(&["issue", "comment", "1", "-F", "-"]), "P"), args(&["issue", "comment", "1", "-F", "P"]));
        assert_eq!(replace_stdin(&args(&["pr", "create", "--body-file=-"]), "P"), args(&["pr", "create", "--body-file=P"]));
        // A lone `-` that is not a file flag's value is left alone.
        assert_eq!(replace_stdin(&args(&["pr", "create", "--title", "-"]), "P"), args(&["pr", "create", "--title", "-"]));
    }

    #[test]
    fn gh_api_fields_and_input_are_text_too() {
        let read = |p: &str| -> Result<Option<String>, ()> { Ok((p == "c.txt").then(|| token())) };
        let s = scan(&args(&["api", "repos/o/r/issues/1/comments", "-f", &format!("body={}", token())]), read, no_stdin);
        assert_eq!(s.hits, vec![Hit { place: "-f body".into(), line: 1, category: "GitHub token" }]);
        let s = scan(&args(&["api", "x", "-F", "body=@c.txt"]), read, no_stdin);
        assert_eq!(s.hits[0].place, "-F body c.txt");
        let s = scan(&args(&["api", "x", "--input", "c.txt"]), read, no_stdin);
        assert_eq!(s.hits.len(), 1);
        // On `gh api`, `-F` is a field, not a body file.
        assert!(!takes_file("-F", true) && takes_file("-F", false));
        // `-F key=@-` reads stdin.
        let a = args(&["api", "x", "-F", "body=@-"]);
        assert!(scan(&a, no_files, no_stdin).uses_stdin);
        assert_eq!(replace_stdin(&a, "P"), args(&["api", "x", "-F", "body=@P"]));
        assert_eq!(replace_stdin(&args(&["api", "x", "--input", "-"]), "P"), args(&["api", "x", "--input", "P"]));
    }

    #[test]
    fn the_refusal_names_where_and_what_kind_and_never_the_text() {
        let a = args(&["pr", "create", "--body", &token()]);
        let s = scan(&a, no_files, no_stdin);
        let text = refusal_text(&a, &s);
        assert!(text.contains("`gh pr create` is refused") && text.contains("--body, line 1  GitHub token"), "{text}");
        assert!(!text.contains("ghp_"), "{text}");
        let big = Scan { too_large: vec!["--body-file huge.md".into()], ..Scan::default() };
        let text = refusal_text(&a, &big);
        assert!(text.contains("can't be checked") && text.contains("too large"), "{text}");
    }
}

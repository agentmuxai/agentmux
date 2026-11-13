// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The text scan: what an agent's `gh` is refused for when the text it sends holds a credential.
//!
//! The push scan (`secret_scan.rs`) looks at the lines a push adds; this looks at the words an
//! agent puts in a pull request, an issue, a comment, a review, release notes or a `gh api` field,
//! with the same formats and the same rule: say where and what kind, never the text itself.
//! Spec: `SPEC_CONNECT_WITH_GITHUB` §5.6, phase G3b.
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
/// What `gh` reads (a body file, stdin) goes to it as a copy in this folder of the temp directory,
/// and copies older than a day are removed.
const COPIES_DIR: &str = "agentmux-gh-text";
const COPIES_KEEP: std::time::Duration = std::time::Duration::from_secs(24 * 60 * 60);

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
        (Some("pr"), Some("create" | "new" | "edit" | "comment" | "review" | "merge" | "close" | "reopen" | "revert")) => true,
        (Some("issue"), Some("create" | "new" | "edit" | "comment" | "close" | "reopen")) => true,
        (Some("release"), Some("create" | "new" | "edit")) => true,
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

/// `-X<value>`: a short flag with its value attached (`-bTEXT`, `-Fpath`, `-fkey=value`), as `-X` and `<value>`.
fn short_attached(arg: &str) -> Option<(&str, &str)> {
    let rest = arg.strip_prefix('-')?;
    let mut chars = rest.chars();
    let letter = chars.next().filter(char::is_ascii_alphabetic)?;
    let value = chars.as_str();
    (!value.is_empty() && !value.starts_with('-')).then(|| (&arg[..1 + letter.len_utf8()], value))
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
        // A flag-like argument is scanned whole too: a credential can be glued to it (`-bTEXT`), or be
        // an argument that merely begins with a dash. A short flag's attached value is its own source.
        if arg.starts_with('-') && arg.len() > 1 {
            if let Some((flag, value)) = short_attached(arg) {
                if takes_file(flag, api) {
                    out.push((flag.to_string(), Source::File(value.to_string())));
                } else if api && matches!(flag, "-F") {
                    push_field(&mut out, flag, value);
                } else if api && matches!(flag, "-f") {
                    let key = value.split_once('=').map(|(k, _)| k).unwrap_or("");
                    out.push((format!("{flag} {key}"), Source::Inline(value.to_string())));
                } else {
                    out.push((flag.to_string(), Source::Inline(value.to_string())));
                }
            }
            if !is_flag(arg) || short_attached(arg).is_some() {
                out.push((prev.map(str::to_string).unwrap_or_else(|| format!("argument {}", i + 1)), Source::Inline(arg.clone())));
            }
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
    /// Every file the command reads (`-` for stdin) with the text scanned. `gh` is given a copy of
    /// each, so it reads exactly what was scanned: stdin and pipes can't be read twice, and a
    /// file could change between the scan and `gh` opening it.
    pub copies: Vec<(String, Vec<u8>)>,
}

impl Scan {
    fn keep_copy(&mut self, original: &str, bytes: Vec<u8>) {
        if !self.copies.iter().any(|(o, _)| o == original) {
            self.copies.push((original.to_string(), bytes));
        }
    }
}

/// Scan `gh <args>`. `read_file` gives a file's text, whatever kind of file it is (a pipe, a
/// device): `Ok(None)` when it isn't there (gh will say so itself), `Err` when it is too large to
/// scan. `stdin` gives what stdin holds, read at most once. The bytes are kept as they are for the
/// copy `gh` is given (a binary upload must arrive unchanged); only the scan reads them as text.
pub fn scan(args: &[String], read_file: impl Fn(&str) -> Result<Option<Vec<u8>>, ()>, mut stdin: impl FnMut() -> Result<Vec<u8>, ()>) -> Scan {
    let mut result = Scan::default();
    for (place, source) in sources(args) {
        match source {
            Source::Inline(text) => scan_text(&place, &text, &mut result.hits),
            Source::File(path) if path == "-" => match stdin() {
                Ok(bytes) => {
                    scan_text(&place, &String::from_utf8_lossy(&bytes), &mut result.hits);
                    result.keep_copy(&path, bytes);
                }
                Err(()) => result.too_large.push(format!("{place} (stdin)")),
            },
            Source::File(path) => match read_file(&path) {
                Ok(Some(bytes)) => {
                    scan_text(&format!("{place} {path}"), &String::from_utf8_lossy(&bytes), &mut result.hits);
                    result.keep_copy(&path, bytes);
                }
                Ok(None) => {}
                Err(()) => result.too_large.push(format!("{place} {path}")),
            },
        }
    }
    result
}

/// `args` with every file the command reads replaced by its copy: `files` maps the value the
/// command gave (`-` for stdin, or a path) to where the copy is.
pub fn replace_files(args: &[String], files: &std::collections::HashMap<String, String>) -> Vec<String> {
    let api = is_api(args);
    // `key=@file` of a `gh api` field.
    let field = |kv: &str| kv.split_once('=').and_then(|(k, v)| files.get(v.strip_prefix('@')?).map(|p| format!("{k}=@{p}")));
    let mut out = Vec::with_capacity(args.len());
    for (i, arg) in args.iter().enumerate() {
        let prev = i.checked_sub(1).map(|p| args[p].as_str()).filter(|p| is_flag(p));
        let after_file_flag = prev.is_some_and(|p| takes_file(p, api));
        let after_field_flag = api && prev.is_some_and(|p| matches!(p, "-F" | "--field"));
        let attached = short_attached(arg).and_then(|(flag, value)| {
            if takes_file(flag, api) {
                files.get(value).map(|p| format!("{flag}{p}"))
            } else if api && flag == "-F" {
                field(value).map(|v| format!("{flag}{v}"))
            } else {
                None
            }
        });
        let replaced = if attached.is_some() {
            attached
        } else if after_file_flag {
            files.get(arg.as_str()).cloned()
        } else if after_field_flag {
            field(arg)
        } else if let Some((flag, value)) = long_flag_value(arg) {
            if takes_file(flag, api) {
                files.get(value).map(|p| format!("{flag}={p}"))
            } else if api && flag == "--field" {
                field(value).map(|v| format!("{flag}={v}"))
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

/// A file's text, whatever it is: a regular file, a named pipe, `/dev/stdin`, a process
/// substitution. `gh` reads those too, so the scan must. `Ok(None)` for a path that isn't there or
/// is a directory (gh says so itself); `Err` when there is more than the scan reads.
fn read_file_limited(path: &str) -> Result<Option<Vec<u8>>, ()> {
    if std::fs::metadata(path).map(|m| m.is_dir()).unwrap_or(false) {
        return Ok(None);
    }
    let Ok(file) = std::fs::File::open(path) else { return Ok(None) };
    let mut buf = Vec::new();
    let _ = file.take(MAX_TEXT_BYTES + 1).read_to_end(&mut buf);
    if buf.len() as u64 > MAX_TEXT_BYTES {
        return Err(());
    }
    Ok(Some(buf))
}

fn read_stdin_limited() -> Result<Vec<u8>, ()> {
    let mut buf = Vec::new();
    let _ = std::io::stdin().lock().take(MAX_TEXT_BYTES + 1).read_to_end(&mut buf);
    if buf.len() as u64 > MAX_TEXT_BYTES {
        return Err(());
    }
    Ok(buf)
}

/// The folder the copies go in: the user's own (mode 0700 on Unix; a folder that is someone
/// else's, or open to others, is not used), with the files older than a day removed.
fn copies_dir() -> Option<std::path::PathBuf> {
    let dir = std::env::temp_dir().join(COPIES_DIR);
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
        std::fs::DirBuilder::new().recursive(true).mode(0o700).create(&dir).ok()?;
        let meta = std::fs::symlink_metadata(&dir).ok()?;
        // SAFETY: geteuid has no preconditions and cannot fail.
        let me = unsafe { libc::geteuid() };
        if !meta.is_dir() || meta.uid() != me || meta.permissions().mode() & 0o077 != 0 {
            return None;
        }
    }
    #[cfg(not(unix))]
    std::fs::create_dir_all(&dir).ok()?;
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for e in entries.flatten() {
            let old = e.metadata().and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).is_some_and(|age| age > COPIES_KEEP);
            if old {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
    Some(dir)
}

/// Write `text` where `gh` can read it as a file.
fn write_copy(dir: &std::path::Path, bytes: &[u8]) -> Option<String> {
    static COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let path = dir.join(format!("{}-{}-{n}.txt", std::process::id(), agentmux_common::time::now_ms()));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(&path).ok()?.write_all(bytes).ok()?;
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
    // Stdin is read once, whichever flags name it.
    let mut stdin_text: Option<Vec<u8>> = None;
    let result = scan(gh_args, read_file_limited, || {
        if let Some(text) = &stdin_text {
            return Ok::<Vec<u8>, ()>(text.clone());
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
    let mut files = std::collections::HashMap::new();
    if !result.copies.is_empty() {
        let dir = copies_dir();
        for (original, bytes) in &result.copies {
            match dir.as_deref().and_then(|d| write_copy(d, bytes)) {
                Some(path) => {
                    files.insert(original.clone(), path);
                }
                None => {
                    eprintln!("agentmux: this gh command is refused: the text it reads was checked but couldn't be handed on after the check.");
                    finish(&[], "refused");
                    return 1;
                }
            }
        }
    }
    let args = replace_files(gh_args, &files);
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
    fn no_files(_: &str) -> Result<Option<Vec<u8>>, ()> {
        Ok(None)
    }
    fn no_stdin() -> Result<Vec<u8>, ()> {
        Ok(Vec::new())
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
            // gh's `new` aliases send the same text.
            &["issue", "new"],
            &["pr", "new", "--body", "x"],
            &["release", "new", "v1"],
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
        let read = |p: &str| -> Result<Option<Vec<u8>>, ()> { Ok((p == "body.md").then(|| format!("# Notes\n{}\n", token()).into_bytes())) };
        let s = scan(&args(&["pr", "create", "--body-file", "body.md"]), read, no_stdin);
        assert_eq!(s.hits, vec![Hit { place: "--body-file body.md".into(), line: 2, category: "GitHub token" }]);
        let s = scan(&args(&["pr", "comment", "1", "-F", "body.md"]), read, no_stdin);
        assert_eq!(s.hits.len(), 1);
        let s = scan(&args(&["pr", "create", "--body-file=body.md"]), read, no_stdin);
        assert_eq!(s.hits.len(), 1);
        let s = scan(&args(&["pr", "create", "--body-file", "nope.md"]), read, no_stdin);
        assert!(s.hits.is_empty() && s.too_large.is_empty());
        // Too large to scan is refused, not waved through.
        let big = |_: &str| -> Result<Option<Vec<u8>>, ()> { Err(()) };
        assert_eq!(scan(&args(&["pr", "create", "--body-file", "huge.md"]), big, no_stdin).too_large.len(), 1);
    }

    fn files(pairs: &[(&str, &str)]) -> std::collections::HashMap<String, String> {
        pairs.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect()
    }

    #[test]
    fn what_gh_reads_is_handed_on_as_a_copy_of_what_was_scanned() {
        // stdin
        let a = args(&["pr", "create", "--title", "t", "--body-file", "-"]);
        let s = scan(&a, no_files, || Ok(format!("x\n{}\n", token()).into_bytes()));
        assert_eq!(s.hits[0].place, "--body-file");
        assert_eq!(s.copies.len(), 1);
        assert_eq!(s.copies[0].0, "-");
        assert_eq!(replace_files(&a, &files(&[("-", "/tmp/b.txt")])), args(&["pr", "create", "--title", "t", "--body-file", "/tmp/b.txt"]));
        assert_eq!(replace_files(&args(&["issue", "comment", "1", "-F", "-"]), &files(&[("-", "P")])), args(&["issue", "comment", "1", "-F", "P"]));
        assert_eq!(replace_files(&args(&["pr", "create", "--body-file=-"]), &files(&[("-", "P")])), args(&["pr", "create", "--body-file=P"]));
        // A lone `-` that is not a file flag's value is left alone.
        assert_eq!(replace_files(&args(&["pr", "create", "--title", "-"]), &files(&[("-", "P")])), args(&["pr", "create", "--title", "-"]));
        // A regular file, a pipe or `/dev/stdin` the same: gh reads the copy, not the original, which
        // can't be read twice (a pipe) or could change after the scan (a file).
        let read = |p: &str| -> Result<Option<Vec<u8>>, ()> { Ok((p == "/dev/stdin" || p == "body.md").then(|| b"clean".to_vec())) };
        let s = scan(&args(&["pr", "create", "--body-file", "/dev/stdin", "-F", "body.md", "-F", "body.md"]), read, no_stdin);
        assert_eq!(s.copies.iter().map(|(o, _)| o.as_str()).collect::<Vec<_>>(), vec!["/dev/stdin", "body.md"]);
        assert_eq!(
            replace_files(&args(&["pr", "create", "--body-file", "/dev/stdin", "-F", "body.md"]), &files(&[("/dev/stdin", "C1"), ("body.md", "C2")])),
            args(&["pr", "create", "--body-file", "C1", "-F", "C2"])
        );
        // A path that isn't there is left alone: gh says so itself.
        assert!(scan(&args(&["pr", "create", "--body-file", "nope"]), no_files, no_stdin).copies.is_empty());
    }

    #[test]
    fn a_binary_file_reaches_gh_byte_for_byte() {
        // Not valid UTF-8 (a binary upload, or a legacy encoding): the scan reads it as text, the copy is the bytes.
        let bytes: Vec<u8> = vec![0x00, 0xff, 0xfe, b'a', 0x80, 0xc3, 0x28];
        let expected = bytes.clone();
        let read = move |p: &str| -> Result<Option<Vec<u8>>, ()> { Ok((p == "asset.bin").then(|| bytes.clone())) };
        let s = scan(&args(&["api", "x", "--input", "asset.bin"]), read, no_stdin);
        assert!(s.hits.is_empty());
        assert_eq!(s.copies, vec![("asset.bin".to_string(), expected)]);
        // `gh pr revert` sends a title and a body too.
        assert!(sends_text(&args(&["pr", "revert", "3"])));
    }

    #[test]
    fn a_repository_flag_with_its_value_glued_on_does_not_hide_the_command() {
        for lead in [&["-Ro/r"][..], &["-R=o/r"], &["--repo=o/r"], &["-R", "o/r"]] {
            let mut a: Vec<&str> = lead.to_vec();
            a.extend(["pr", "create", "--title", "x"]);
            let a: Vec<String> = a.iter().map(|s| s.to_string()).collect();
            assert!(sends_text(&a), "{lead:?}");
        }
    }

    #[test]
    fn a_short_flag_with_its_value_glued_on_is_scanned_too() {
        let aws = format!("{}{}", "AKIA", "ABCDEFGHIJKLMNOP");
        for a in [
            args(&["pr", "create", &format!("-t{aws}")]),
            args(&["pr", "create", &format!("-b{aws}")]),
            args(&["pr", "create", &format!("-b{}", token())]),
            args(&["issue", "comment", "1", &format!("-sk-ant-{}", "api03-abcdefghijklmnopqrstuvwxyz0123456789")]),
            args(&["api", "x", &format!("-fbody={aws}")]),
            args(&["api", "x", &format!("--raw-field=body={aws}")]),
        ] {
            assert!(!scan(&a, no_files, no_stdin).hits.is_empty(), "{a:?}");
        }
        // The file flags with the path glued on name a file to read and copy.
        let read = |p: &str| -> Result<Option<Vec<u8>>, ()> { Ok((p == "b.md").then(|| aws.clone().into_bytes())) };
        let a = args(&["pr", "create", "-Fb.md"]);
        let s = scan(&a, read, no_stdin);
        assert_eq!((s.hits.len(), s.copies.len()), (1, 1));
        assert_eq!(replace_files(&a, &files(&[("b.md", "C")])), args(&["pr", "create", "-FC"]));
        let a = args(&["api", "x", "-Fbody=@b.md"]);
        assert_eq!(scan(&a, read, no_stdin).hits.len(), 1);
        assert_eq!(replace_files(&a, &files(&[("b.md", "C")])), args(&["api", "x", "-Fbody=@C"]));
        // Plain flags are not text.
        assert!(scan(&args(&["pr", "create", "--draft", "--fill", "-d", "-R", "o/r"]), no_files, no_stdin).hits.is_empty());
    }

    #[test]
    fn a_file_of_any_kind_is_read_for_the_scan() {
        let dir = std::env::temp_dir().join(format!("amx-gh-text-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("b.md");
        std::fs::write(&file, format!("{}
", token())).unwrap();
        let bytes = read_file_limited(file.to_str().unwrap()).unwrap().unwrap();
        assert!(credential_in(&String::from_utf8_lossy(&bytes)).is_some());
        // A directory and a missing path are gh's to report.
        assert_eq!(read_file_limited(dir.to_str().unwrap()), Ok(None));
        assert_eq!(read_file_limited(dir.join("nope").to_str().unwrap()), Ok(None));
        #[cfg(unix)]
        {
            // A device, which is not a regular file, is read too (and `/dev/null` is empty).
            assert_eq!(read_file_limited("/dev/null"), Ok(Some(Vec::new())));
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn gh_api_fields_and_input_are_text_too() {
        let read = |p: &str| -> Result<Option<Vec<u8>>, ()> { Ok((p == "c.txt").then(|| token().into_bytes())) };
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
        assert_eq!(scan(&a, no_files, no_stdin).copies.len(), 1);
        assert_eq!(replace_files(&a, &files(&[("-", "P")])), args(&["api", "x", "-F", "body=@P"]));
        assert_eq!(replace_files(&args(&["api", "x", "--field", "b=@c.txt"]), &files(&[("c.txt", "P")])), args(&["api", "x", "--field", "b=@P"]));
        assert_eq!(replace_files(&args(&["api", "x", "--field=b=@c.txt"]), &files(&[("c.txt", "P")])), args(&["api", "x", "--field=b=@P"]));
        assert_eq!(replace_files(&args(&["api", "x", "--input", "-"]), &files(&[("-", "P")])), args(&["api", "x", "--input", "P"]));
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

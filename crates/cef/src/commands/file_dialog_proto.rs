// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What the host and its file-dialog helper process say to each other (#196, Linux).
//!
//! On Linux the desktop portal refuses a process that is not dumpable, and the host is not on purpose
//! (`process_hardening`), so the host cannot open a portal dialog itself. It starts a short-lived child of its
//! own executable instead (`--internal-file-dialog`, see `app::file_dialog_helper`), which is not hardened,
//! shows the dialog and prints the one path the person chose. The child loads no CEF, no app state, no settings
//! and no Vault: it gets the dialog's typed fields on its command line and returns one line on stdout.
//!
//! This module is that contract and nothing else, plain data and functions so it is tested on every platform:
//! the fields as an argv, the argv back as fields, and the host's reading of the answer. The host never trusts
//! the answer: one line, an absolute path, no `..`, a size cap; anything else is a failure, not a choice.

use std::path::{Component, Path, PathBuf};

/// The flag that makes a re-exec of the host a dialog helper. Checked first thing in `run()`.
pub(crate) const HELPER_FLAG: &str = "--internal-file-dialog";

/// The longest answer line the host reads. A path is at most `PATH_MAX` (4096) bytes.
pub(crate) const MAX_ANSWER_BYTES: usize = 4096;

/// Exit code of a helper whose person closed the dialog with nothing chosen.
pub(crate) const EXIT_CANCELLED: i32 = 1;
/// Exit code of a helper that could not show a dialog at all (bad fields, no backend).
pub(crate) const EXIT_FAILED: i32 = 2;

/// What the dialog is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub(crate) enum Kind {
    #[default]
    Save,
    Open,
}

/// One filter of the dialog: a label and the extensions it offers, without the dot (`*` for any).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Filter {
    pub label: String,
    pub extensions: Vec<String>,
}

/// Everything a dialog needs to be shown, as typed fields (never a string from a pane run through a shell).
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub(crate) struct DialogSpec {
    pub kind: Kind,
    pub title: String,
    /// The file name offered (Save).
    pub file_name: Option<String>,
    /// The folder the dialog starts in.
    pub dir: Option<PathBuf>,
    pub filters: Vec<Filter>,
}

impl DialogSpec {
    pub(crate) fn new(kind: Kind, title: impl Into<String>) -> Self {
        Self { kind, title: title.into(), ..Self::default() }
    }

    pub(crate) fn file_name(mut self, name: impl Into<String>) -> Self {
        self.file_name = Some(name.into());
        self
    }

    pub(crate) fn dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.dir = Some(dir.into());
        self
    }

    pub(crate) fn filter(mut self, label: &str, extensions: &[&str]) -> Self {
        self.filters.push(Filter { label: label.to_string(), extensions: extensions.iter().map(|e| e.to_string()).collect() });
        self
    }

    /// The helper's argument list: the flag, then one `--key=value` per field. Built from typed fields, so a value
    /// can hold any character and never changes what the other arguments mean.
    pub(crate) fn to_args(&self) -> Vec<String> {
        let mut args = vec![HELPER_FLAG.to_string()];
        args.push(format!("--kind={}", if self.kind == Kind::Open { "open" } else { "save" }));
        args.push(format!("--title={}", self.title));
        if let Some(name) = &self.file_name {
            args.push(format!("--name={name}"));
        }
        if let Some(dir) = &self.dir {
            args.push(format!("--dir={}", dir.to_string_lossy()));
        }
        for f in &self.filters {
            args.push(format!("--filter={}{FILTER_SEP}{}", f.label, f.extensions.join(",")));
        }
        args
    }

    /// The fields from the helper's arguments (everything after the program name). Refuses an argument it does not
    /// know, so a mistaken call is a failure and not a dialog with half its fields.
    pub(crate) fn from_args<I: IntoIterator<Item = String>>(args: I) -> Result<Self, String> {
        let mut spec = Self::default();
        let mut saw_kind = false;
        for arg in args {
            if arg == HELPER_FLAG {
                continue;
            }
            let Some((key, value)) = arg.strip_prefix("--").and_then(|a| a.split_once('=')) else {
                return Err("an argument that is not --key=value".into());
            };
            match key {
                "kind" => {
                    spec.kind = match value {
                        "save" => Kind::Save,
                        "open" => Kind::Open,
                        _ => return Err("an unknown kind".into()),
                    };
                    saw_kind = true;
                }
                "title" => spec.title = value.to_string(),
                "name" => spec.file_name = Some(value.to_string()),
                "dir" => spec.dir = Some(PathBuf::from(value)),
                "filter" => {
                    let (label, exts) = value.split_once(FILTER_SEP).ok_or("a filter with no extensions part")?;
                    let extensions = exts.split(',').filter(|e| !e.is_empty()).map(str::to_string).collect();
                    spec.filters.push(Filter { label: label.to_string(), extensions });
                }
                _ => return Err("an unknown argument".into()),
            }
        }
        if saw_kind { Ok(spec) } else { Err("no kind given".into()) }
    }
}

/// Between a filter's label and its extensions: the ASCII unit separator, which no label holds.
const FILTER_SEP: char = '\u{1f}';

/// The host's reading of what the helper printed. Exactly one line holding one absolute path with no `..` in it,
/// at most [`MAX_ANSWER_BYTES`]; the path is returned normalised (`.` parts dropped). Anything else is `Err` with a
/// reason in words: the host treats it as a failed dialog, never as a choice.
pub(crate) fn parse_answer(out: &[u8]) -> Result<PathBuf, String> {
    if out.len() > MAX_ANSWER_BYTES {
        return Err("the answer was too long".into());
    }
    let text = std::str::from_utf8(out).map_err(|_| "the answer was not text")?;
    let line = text.strip_suffix('\n').unwrap_or(text);
    let line = line.strip_suffix('\r').unwrap_or(line);
    if line.is_empty() {
        return Err("the answer was empty".into());
    }
    if line.contains(['\n', '\r', '\0']) {
        return Err("the answer was more than one line".into());
    }
    let path = Path::new(line);
    if !path.is_absolute() {
        return Err("the answer was not an absolute path".into());
    }
    if path.components().any(|c| c == Component::ParentDir) {
        return Err("the answer had a .. in it".into());
    }
    // `components` drops the `.` parts and doubled separators.
    Ok(path.components().collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn abs(name: &str) -> PathBuf {
        std::env::temp_dir().join(name)
    }

    #[test]
    fn the_fields_survive_a_trip_through_the_arguments() {
        let spec = DialogSpec::new(Kind::Save, "Save as")
            .file_name("a b=c --d.txt")
            .dir(abs("My Folder"))
            .filter("Vault, \"files\"", &["vault", "kdbx"])
            .filter("All files", &["*"]);
        let back = DialogSpec::from_args(spec.to_args()).unwrap();
        assert_eq!(back, spec);
        // An open dialog with nothing else set.
        let open = DialogSpec::new(Kind::Open, "");
        assert_eq!(DialogSpec::from_args(open.to_args()).unwrap(), open);
    }

    #[test]
    fn a_value_that_looks_like_a_flag_stays_a_value() {
        let spec = DialogSpec::new(Kind::Save, "--kind=open").file_name("--internal-file-dialog");
        let back = DialogSpec::from_args(spec.to_args()).unwrap();
        assert_eq!(back.kind, Kind::Save);
        assert_eq!(back.title, "--kind=open");
        assert_eq!(back.file_name.as_deref(), Some("--internal-file-dialog"));
    }

    #[test]
    fn mistaken_arguments_are_refused_and_not_half_read() {
        assert!(DialogSpec::from_args(["--title=x".to_string()]).is_err(), "no kind");
        assert!(DialogSpec::from_args(["--kind=save".to_string(), "--bogus=1".to_string()]).is_err());
        assert!(DialogSpec::from_args(["--kind=sideways".to_string()]).is_err());
        assert!(DialogSpec::from_args(["--kind=save".to_string(), "loose".to_string()]).is_err());
        assert!(DialogSpec::from_args(["--kind=save".to_string(), "--filter=no-separator".to_string()]).is_err());
    }

    #[test]
    fn one_absolute_path_on_one_line_is_an_answer() {
        let p = abs("saved.txt");
        let line = format!("{}\n", p.display());
        assert_eq!(parse_answer(line.as_bytes()).unwrap(), p);
        // No newline at the end is fine too, and a doubled separator or `.` part is tidied.
        assert_eq!(parse_answer(p.to_string_lossy().as_bytes()).unwrap(), p);
        let odd = format!("{}{}.{}saved.txt", std::env::temp_dir().display(), std::path::MAIN_SEPARATOR, std::path::MAIN_SEPARATOR);
        assert_eq!(parse_answer(odd.as_bytes()).unwrap(), p);
    }

    #[test]
    fn anything_else_is_a_failure_and_not_a_choice() {
        assert!(parse_answer(b"").is_err(), "empty");
        assert!(parse_answer(b"\n").is_err(), "an empty line");
        assert!(parse_answer(b"relative/file.txt\n").is_err(), "relative");
        let p = abs("a.txt");
        let two = format!("{}\n{}\n", p.display(), p.display());
        assert!(parse_answer(two.as_bytes()).is_err(), "two lines");
        let up = format!("{}{}..{}x", std::env::temp_dir().display(), std::path::MAIN_SEPARATOR, std::path::MAIN_SEPARATOR);
        assert!(parse_answer(up.as_bytes()).is_err(), "a .. part");
        let huge = vec![b'a'; MAX_ANSWER_BYTES + 1];
        assert!(parse_answer(&huge).is_err(), "too long");
        assert!(parse_answer(&[0xff, 0xfe, b'\n']).is_err(), "not text");
        let nul = format!("{}\0x\n", p.display());
        assert!(parse_answer(nul.as_bytes()).is_err(), "a NUL");
    }
}

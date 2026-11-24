// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `fs.link_file_check`: whether a file a message names is in its author's folders, so a window can draw it as a link
//! or as a muted word (docs/specs/SPEC_LINKS_ON_EVERYTHING_2026_11_20.md §5, §6.4).
//!
//! Built to be safe on text full of hostile paths:
//! - **Read-only.** It looks whether a file exists and reads nothing.
//! - **Canonicalises before it compares.** Symlinks, `..`, junctions and short names are resolved first, and the
//!   result, not the written path, is compared with the author's folder.
//! - **Rate-limited** per author and overall, so text full of paths can't be used to probe the disk.
//! - **Says nothing about what is outside.** A path outside the author's folders answers `outside` whether or not
//!   anything is there, and no answer names a host path.
//!
//! A container agent's paths are resolved by `agent_media_files::resolve`, as a click on them is. Any other agent's
//! are resolved against the pane's working directory.

use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use crate::backend::obj::{Block, MetaMapType};
use crate::backend::rpc_types::{FsLinkFileCheck, FsLinkFileReq};

use super::AppState;

const WINDOW: Duration = Duration::from_secs(10);
const PER_AUTHOR: u32 = 120;
const OVERALL: u32 = 600;

const FOUND: &str = "found";
const MISSING: &str = "missing";
const OUTSIDE: &str = "outside";
const LIMITED: &str = "limited";

const NOT_FOUND_WORDS: &str = "Not found.";
const OUTSIDE_WORDS: &str = "Outside this agent's folders.";
const FOLDER_WORDS: &str = "That is a folder, not a file.";
const LIMITED_WORDS: &str = "Too many files were checked at once. Hover it again in a moment.";

fn answer(state: &str, reason: &str) -> FsLinkFileCheck {
    FsLinkFileCheck { state: state.to_string(), reason: reason.to_string(), resolved: String::new() }
}

/// Asks counted in the current window, per author and overall.
#[derive(Default)]
struct Counts {
    since: Option<Instant>,
    by_author: HashMap<String, u32>,
    overall: u32,
}

impl Counts {
    fn allow(&mut self, author: &str, now: Instant) -> bool {
        if self.since.is_none_or(|since| now.duration_since(since) >= WINDOW) {
            *self = Counts { since: Some(now), ..Counts::default() };
        }
        let mine = self.by_author.entry(author.to_string()).or_default();
        if *mine >= PER_AUTHOR || self.overall >= OVERALL {
            return false;
        }
        *mine += 1;
        self.overall += 1;
        true
    }
}

fn counts() -> &'static Mutex<Counts> {
    static COUNTS: OnceLock<Mutex<Counts>> = OnceLock::new();
    COUNTS.get_or_init(Mutex::default)
}

/// `path` with `.` and `..` taken out, without touching the disk; `None` when a `..` would leave its root.
fn collapse(path: &Path) -> Option<PathBuf> {
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            Component::ParentDir => {
                if !out.pop() {
                    return None;
                }
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    Some(out)
}

/// How many links in a row are followed by hand.
const MAX_HOPS: usize = 16;

/// Whether a link on the way down `path` leads out of the folder (`inside`: the folder as written and as resolved),
/// whether what it points at is there or not, and whatever follows it. A path through such a link answers as one that
/// leaves the folder does: what lies outside is not told by whether a file is there.
pub(crate) fn dangling_link_leaves(path: &Path, inside: &[&Path]) -> bool {
    let mut at = PathBuf::new();
    for part in path.components() {
        at.push(part.as_os_str());
        // The folder's own place (a link above it, such as a short name of a data disk) is where the agent lives, not a way out of it.
        if inside.iter().any(|root| root.starts_with(&at)) {
            continue;
        }
        let mut here = at.clone();
        for _ in 0..MAX_HOPS {
            let Ok(meta) = std::fs::symlink_metadata(&here) else { break };
            if !meta.file_type().is_symlink() {
                break;
            }
            // A link that resolves leaves the folder when what it resolves to is not under it, whatever follows it.
            if let Ok(resolved) = crate::server::browser_uploads::canonical(&here) {
                if !inside.iter().any(|root| resolved.starts_with(root)) {
                    return true;
                }
                break;
            }
            let Ok(target) = std::fs::read_link(&here) else { break };
            let joined = if target.is_absolute() { target } else { here.parent().map_or(target.clone(), |parent| parent.join(&target)) };
            let Some(next) = collapse(&joined) else { return true };
            if !inside.iter().any(|root| next.starts_with(root)) {
                return true;
            }
            here = next;
        }
    }
    false
}

/// A file named in a message of a host agent whose pane works in `cwd`: found when it is a file inside that folder.
fn check_in_folder(cwd: &str, asked: &str) -> FsLinkFileCheck {
    // A default agent folder is stored as `~/.agentmux/agents/<slug>`: the pane's own folder, expanded.
    let cwd = crate::backend::base::expand_home_dir_safe(cwd).to_string_lossy().into_owned();
    let cwd = cwd.as_str();
    let asked = asked.trim();
    if asked.is_empty() || asked.contains('\0') || asked.starts_with('~') || asked.starts_with("\\\\") || asked.starts_with("//") || cwd.trim().is_empty() {
        return answer(OUTSIDE, OUTSIDE_WORDS);
    }
    let Ok(root) = crate::server::browser_uploads::canonical(Path::new(cwd)) else { return answer(OUTSIDE, OUTSIDE_WORDS) };
    let written = Path::new(asked);
    let candidate = if written.is_absolute() { written.to_path_buf() } else { Path::new(cwd).join(written) };
    // Where the path leads on the page, before the disk is asked: a `..` that leaves the folder never reaches it.
    let lexically_inside = collapse(&candidate).is_some_and(|c| c.starts_with(Path::new(cwd)) || c.starts_with(&root));
    match crate::server::browser_uploads::canonical(&candidate) {
        Ok(file) if file.starts_with(&root) => {
            if file.is_file() {
                FsLinkFileCheck { resolved: file.to_string_lossy().into_owned(), ..answer(FOUND, "") }
            } else {
                answer(MISSING, FOLDER_WORDS)
            }
        }
        // It exists, and leads out of the folder (a link, a junction, a short name of something else).
        Ok(_) => answer(OUTSIDE, OUTSIDE_WORDS),
        // A link on the way leads out of the folder and its target is not there: as outside as one whose target is.
        Err(_) if dangling_link_leaves(&candidate, &[Path::new(cwd), &root]) => answer(OUTSIDE, OUTSIDE_WORDS),
        Err(_) if lexically_inside => answer(MISSING, NOT_FOUND_WORDS),
        // Not there, and not in the folder: nothing is said about whether it would have been.
        Err(_) => answer(OUTSIDE, OUTSIDE_WORDS),
    }
}

/// A file named in a message of the container agent whose pane is `meta`, found by the same rules a click on it uses.
fn check_in_container(meta: &MetaMapType, asked: &str) -> FsLinkFileCheck {
    match crate::server::agent_media_files::resolve(meta, asked) {
        Ok(found) if found.file.is_file() => answer(FOUND, ""),
        Ok(_) => answer(MISSING, FOLDER_WORDS),
        Err(why) if why.contains("doesn't exist") || why.contains("can't be read") => answer(MISSING, NOT_FOUND_WORDS),
        Err(_) => answer(OUTSIDE, OUTSIDE_WORDS),
    }
}

/// `fs.link_file_check`.
pub(crate) async fn fs_link_file_check(state: &AppState, req: FsLinkFileReq) -> Result<FsLinkFileCheck, String> {
    let allowed = counts().lock().map_or(true, |mut c| c.allow(&req.block_id, Instant::now()));
    if !allowed {
        return Ok(answer(LIMITED, LIMITED_WORDS));
    }
    let Some(block) = state.mstore.get::<Block>(&req.block_id).ok().flatten() else {
        return Ok(answer(OUTSIDE, OUTSIDE_WORDS));
    };
    let container = crate::backend::runtime_adapter::RuntimeKind::of_block_meta(&block.meta).is_container();
    let path = req.path;
    tokio::task::spawn_blocking(move || {
        if container {
            check_in_container(&block.meta, &path)
        } else {
            check_in_folder(&crate::backend::obj::meta_get_string(&block.meta, "cmd:cwd", ""), &path)
        }
    })
    .await
    .map_err(|e| format!("filesystem task failed: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("work")).unwrap();
        std::fs::create_dir(dir.path().join("work").join("notes")).unwrap();
        std::fs::write(dir.path().join("work").join("notes").join("plan.md"), "x").unwrap();
        std::fs::write(dir.path().join("secret.txt"), "x").unwrap();
        dir
    }

    fn check(dir: &tempfile::TempDir, asked: &str) -> (String, String) {
        let cwd = dir.path().join("work");
        let got = check_in_folder(&cwd.to_string_lossy(), asked);
        (got.state, got.reason)
    }

    #[test]
    fn a_file_in_the_folder_is_found_by_a_relative_or_an_absolute_path() {
        let dir = folder();
        assert_eq!(check(&dir, "notes/plan.md").0, FOUND);
        let found = check_in_folder(&dir.path().join("work").to_string_lossy(), "notes/plan.md");
        assert!(found.resolved.ends_with("plan.md"), "a found host file says where it is for the window to open");
        assert!(check_in_folder(&dir.path().join("work").to_string_lossy(), "notes/none.md").resolved.is_empty());
        assert_eq!(check(&dir, "./notes/../notes/plan.md").0, FOUND);
        let absolute = dir.path().join("work").join("notes").join("plan.md");
        assert_eq!(check(&dir, &absolute.to_string_lossy()).0, FOUND);
    }

    #[test]
    fn a_missing_file_in_the_folder_is_missing_and_a_folder_is_not_a_file() {
        let dir = folder();
        assert_eq!(check(&dir, "notes/none.md"), (MISSING.to_string(), NOT_FOUND_WORDS.to_string()));
        assert_eq!(check(&dir, "notes"), (MISSING.to_string(), FOLDER_WORDS.to_string()));
    }

    #[test]
    fn a_dot_dot_escape_comes_back_outside_whether_or_not_the_target_exists() {
        let dir = folder();
        for asked in ["../secret.txt", "../nothing-here.txt", "notes/../../secret.txt", "../../../../etc/hosts"] {
            assert_eq!(check(&dir, asked), (OUTSIDE.to_string(), OUTSIDE_WORDS.to_string()), "{asked}");
        }
    }

    #[test]
    fn an_absolute_path_elsewhere_and_a_home_or_network_path_are_outside() {
        let dir = folder();
        let elsewhere = dir.path().join("secret.txt");
        assert_eq!(check(&dir, &elsewhere.to_string_lossy()).0, OUTSIDE);
        for asked in ["~/notes.md", "//host/share/a.md", "\\\\host\\share\\a.md", "", "a\0b.md"] {
            assert_eq!(check(&dir, asked).0, OUTSIDE, "{asked:?}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_link_inside_the_folder_that_leads_out_comes_back_outside() {
        let dir = folder();
        let work = dir.path().join("work");
        std::os::unix::fs::symlink(dir.path().join("secret.txt"), work.join("shortcut.txt")).unwrap();
        std::os::unix::fs::symlink(dir.path(), work.join("up")).unwrap();
        assert_eq!(check(&dir, "shortcut.txt"), (OUTSIDE.to_string(), OUTSIDE_WORDS.to_string()));
        assert_eq!(check(&dir, "up/secret.txt").0, OUTSIDE);
    }

    #[cfg(unix)]
    #[test]
    fn a_link_that_leaves_the_folder_answers_outside_whether_or_not_its_target_is_there() {
        let dir = folder();
        let work = dir.path().join("work");
        let link = |name: &str, to: &std::path::Path| std::os::unix::fs::symlink(to, work.join(name)).unwrap();
        link("gone.txt", &dir.path().join("not-there.txt"));
        link("gone-dir", &dir.path().join("not-there-dir"));
        link("present.txt", &dir.path().join("secret.txt"));
        link("relative", std::path::Path::new("../not-there.txt"));
        link("chain-b", &dir.path().join("not-there.txt"));
        link("chain-a", &work.join("chain-b"));
        link("up", dir.path());
        for asked in ["gone.txt", "gone-dir/a.md", "gone-dir", "present.txt", "relative", "chain-a", "gone-dir/../plan.md", "up/nothing.txt", "up/secret.txt", "up/nothing/deeper.txt"] {
            assert_eq!(check(&dir, asked), (OUTSIDE.to_string(), OUTSIDE_WORDS.to_string()), "{asked}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_link_inside_the_folder_whose_target_is_not_there_is_missing() {
        let dir = folder();
        let work = dir.path().join("work");
        std::os::unix::fs::symlink(work.join("notes").join("later.md"), work.join("soon.md")).unwrap();
        std::os::unix::fs::symlink("notes/later.md", work.join("soon-relative.md")).unwrap();
        assert_eq!(check(&dir, "soon.md"), (MISSING.to_string(), NOT_FOUND_WORDS.to_string()));
        assert_eq!(check(&dir, "soon-relative.md"), (MISSING.to_string(), NOT_FOUND_WORDS.to_string()));
        // A link to a folder of its own whose tail is not there is only a file that is not there.
        std::os::unix::fs::symlink(work.join("notes"), work.join("inside-dir")).unwrap();
        assert_eq!(check(&dir, "inside-dir/later.md"), (MISSING.to_string(), NOT_FOUND_WORDS.to_string()));
        assert_eq!(check(&dir, "inside-dir/plan.md").0, FOUND);
    }

    #[cfg(unix)]
    #[test]
    fn a_folder_reached_through_a_link_above_it_answers_missing_for_a_file_that_is_not_there() {
        let dir = folder();
        let door = dir.path().join("door");
        std::os::unix::fs::symlink(dir.path(), &door).unwrap();
        let cwd = door.join("work").to_string_lossy().into_owned();
        let state = |asked: &str| check_in_folder(&cwd, asked).state;
        assert_eq!(state("notes/none.md"), MISSING);
        assert_eq!(state("notes/plan.md"), FOUND);
        assert_eq!(state("../secret.txt"), OUTSIDE);
        // A link beneath the folder that leaves it is still outside.
        std::os::unix::fs::symlink(dir.path(), dir.path().join("work").join("up")).unwrap();
        assert_eq!(state("up/nothing.txt"), OUTSIDE);
    }

    #[test]
    fn no_answer_names_a_host_path() {
        let dir = folder();
        let host = dir.path().to_string_lossy().to_string();
        for asked in ["notes/plan.md", "notes/none.md", "../secret.txt", "notes"] {
            let (state, reason) = check(&dir, asked);
            assert!(!reason.contains(&host) && !state.contains(&host), "{asked}");
        }
    }

    #[test]
    fn a_working_folder_stored_with_a_home_prefix_is_the_folder_it_names() {
        // `~/` is the person's home, which exists: a file that is not there is missing, not outside.
        let got = check_in_folder("~/", "wren-no-such-file-for-this-test.md");
        assert_eq!((got.state.as_str(), got.reason.as_str()), (MISSING, NOT_FOUND_WORDS));
        assert_eq!(check_in_folder("~/", "../wren-no-such-file-for-this-test.md").state, OUTSIDE, "a path leaving the folder is still outside");
        assert_eq!(check_in_folder("~user/x", "a.md").state, OUTSIDE, "another user's home is not a folder this pane has");
    }

    #[test]
    fn a_pane_with_no_working_folder_checks_nothing() {
        assert_eq!(check_in_folder("", "a.md").state, OUTSIDE);
    }

    #[test]
    fn the_checks_are_limited_per_author_and_overall() {
        let mut counts = Counts::default();
        let now = Instant::now();
        assert!((0..PER_AUTHOR).all(|_| counts.allow("a", now)));
        assert!(!counts.allow("a", now), "the author's share is used up");
        assert!(counts.allow("b", now), "another author is not held up by it");
        assert!(counts.allow("a", now + WINDOW), "a new window starts afresh");
        let mut counts = Counts::default();
        let allowed = (0..OVERALL + 10).filter(|i| counts.allow(&format!("author-{i}"), now)).count();
        assert_eq!(allowed, OVERALL as usize);
    }
}

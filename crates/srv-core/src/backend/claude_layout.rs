// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Where the Claude Code CLI keeps a conversation under a `CLAUDE_CONFIG_DIR`.
//!
//! One implementation, because every reader has to agree with the CLI byte for
//! byte: a resume check that computes a different folder name than the CLI
//! reports a transcript that is sitting on disk as unreachable, and the agent
//! loses its native resume. Three copies of this rule used to disagree, and
//! the one on the resume path was wrong for any working directory holding an
//! underscore, a space or most other punctuation.

/// The CLI keeps this many characters of the sanitized path before it
/// appends a hash instead.
const PROJECT_DIR_NAME_MAX: usize = 200;

/// The folder under `<CLAUDE_CONFIG_DIR>/projects/` that holds the sessions
/// the CLI ran with `cwd` as its working directory.
///
/// The CLI's rule (its `RC()`, read from the binary AgentMux 0.57.0 bundles,
/// Claude Code v2.1.280):
///
/// ```js
/// function TQ(e){let r=0;for(let n=0;n<e.length;n++)r=(r<<5)-r+e.charCodeAt(n)|0;return r}
/// function RC(e){let n=e.replace(/[^a-zA-Z0-9]/g,"-");
///   if(n.length<=200)return n;return`${n.slice(0,200)}-${Math.abs(TQ(e)).toString(36)}`}
/// ```
///
/// JavaScript strings are UTF-16, so the rule works on UTF-16 code units: a
/// character outside the Basic Multilingual Plane (an emoji) becomes two
/// dashes, not one.
///
/// `cwd` must be the path as the CLI sees it — already expanded, not a
/// `~`-shorthand.
pub fn project_dir_name(cwd: &str) -> String {
    let name: String = cwd
        .encode_utf16()
        .map(|unit| match u8::try_from(unit) {
            Ok(b) if b.is_ascii_alphanumeric() => b as char,
            _ => '-',
        })
        .collect();
    if name.len() <= PROJECT_DIR_NAME_MAX {
        return name;
    }
    // Every character of `name` is ASCII, so slicing by bytes is slicing by
    // characters.
    format!(
        "{}-{}",
        &name[..PROJECT_DIR_NAME_MAX],
        radix_36(path_hash(cwd))
    )
}

/// `Math.abs(TQ(cwd))`: 31·h + c over UTF-16 code units, wrapping at 32 bits
/// (Java's `String.hashCode`). `i32::MIN`'s absolute value doesn't fit an
/// `i32`, and JavaScript numbers don't wrap, so the magnitude is returned
/// unsigned.
fn path_hash(s: &str) -> u32 {
    let mut hash: i32 = 0;
    for unit in s.encode_utf16() {
        hash = hash
            .wrapping_shl(5)
            .wrapping_sub(hash)
            .wrapping_add(i32::from(unit));
    }
    hash.unsigned_abs()
}

/// `Number.prototype.toString(36)` for a non-negative integer.
fn radix_36(mut n: u32) -> String {
    const DIGITS: &[u8; 36] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    if n == 0 {
        return "0".to_string();
    }
    let mut buf = Vec::new();
    while n > 0 {
        buf.push(DIGITS[(n % 36) as usize]);
        n /= 36;
    }
    buf.reverse();
    String::from_utf8(buf).expect("base-36 digits are ASCII")
}

/// The directory the CLI names a working directory's auto-memory folder
/// after — not the working directory itself, unlike its sessions. The CLI's
/// rule (its `defaultPath()`, Claude Code v2.1.280):
///
/// ```js
/// g = this.canonicalWcRootForProject(r) ?? Ur(r) ?? r   // r: the process cwd
/// function Ur(e){let n=Un(e);if(!n)return null;return eQ(uu().canonicalRootByRoot,n,ke)}
/// ```
///
/// That is: the physical working directory, walked up to the nearest `.git`
/// (a directory or a file), and — for a linked worktree, whose `.git` is a
/// `gitdir:` file — on to the main checkout that owns it. So every
/// subdirectory and every linked worktree of one repository shares the
/// repository root's memory folder.
///
/// Returns the working directory unchanged when it isn't in a repository.
///
/// A folder a container agent can write is named after its own path alone:
/// that agent's memory is a folder AgentMux binds into its container, so
/// nothing the agent can write decides which one it is. No file in the
/// folder is read, no folder above it is looked at, and a link in a part of
/// the path such an agent can write is not followed. A linked worktree whose
/// repository such an agent can write keeps its own root, for the same reason.
pub fn memory_project_root(cwd: &std::path::Path) -> std::path::PathBuf {
    memory_project_root_with(cwd, &crate::backend::fs_ops::container_folders::written_by_a_container_agent)
}

/// [`memory_project_root`], with `written` saying whether a container agent
/// can write a folder.
pub fn memory_project_root_with(cwd: &std::path::Path, written: &dyn Fn(&std::path::Path) -> bool) -> std::path::PathBuf {
    if written(cwd) {
        // Inside another such folder, a part of its own path can be written too.
        return if cwd.parent().is_some_and(written) { as_written(cwd) } else { physical_dir(cwd) };
    }
    let cwd = physical_dir(cwd);
    match git_root(&cwd) {
        Some(root) => canonical_worktree_root(&root, written),
        None => cwd,
    }
}

/// `dir` as it is written: absolute, with `.` and `..` taken out, and nothing
/// looked up on the disk.
fn as_written(dir: &std::path::Path) -> std::path::PathBuf {
    use std::path::Component;
    let absolute = std::path::absolute(dir).unwrap_or_else(|_| dir.to_path_buf());
    let mut out = std::path::PathBuf::new();
    for part in absolute.components() {
        match part {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

/// The working directory as the CLI's process sees it (`process.cwd()`):
/// symlinks resolved. On Windows `canonicalize` returns a `\\?\` verbatim
/// path the CLI never sees, and the current directory keeps links as given.
pub fn physical_dir(cwd: &std::path::Path) -> std::path::PathBuf {
    if cfg!(windows) {
        cwd.to_path_buf()
    } else {
        cwd.canonicalize().unwrap_or_else(|_| cwd.to_path_buf())
    }
}

/// Whether the repository whose main checkout is `root` has linked
/// worktrees — each of which shares `root`'s memory folder.
pub fn has_linked_worktrees(root: &std::path::Path) -> bool {
    std::fs::read_dir(root.join(".git").join("worktrees")).is_ok_and(|mut d| d.next().is_some())
}

/// The nearest directory at or above `dir` holding a `.git` directory or
/// file (the CLI's `Gt`).
fn git_root(dir: &std::path::Path) -> Option<std::path::PathBuf> {
    dir.ancestors()
        .find(|d| std::fs::metadata(d.join(".git")).is_ok_and(|m| m.is_dir() || m.is_file()))
        .map(std::path::Path::to_path_buf)
}

/// For a linked worktree, the main checkout that owns it; otherwise `root`
/// (the CLI's `ke`/`Ht`). Anything that doesn't check out — a `.git` file
/// that isn't `gitdir:`, a missing `commondir`, a worktree whose `gitdir`
/// doesn't point back — leaves `root` as it is, as the CLI does. So does a
/// repository a container agent can write (`written`).
fn canonical_worktree_root(root: &std::path::Path, written: &dyn Fn(&std::path::Path) -> bool) -> std::path::PathBuf {
    use std::path::{Path, PathBuf};
    fn read_trimmed(p: &Path) -> Option<String> {
        std::fs::read_to_string(p).ok().map(|s| s.trim().to_string())
    }
    let resolve = || -> Option<PathBuf> {
        let git_file = read_trimmed(&root.join(".git"))?;
        let git_dir = real_path(&root.join(git_file.strip_prefix("gitdir:")?.trim()));
        if written(&git_dir) {
            return None;
        }
        let common = real_path(&git_dir.join(read_trimmed(&git_dir.join("commondir"))?));
        if written(&common) {
            return None;
        }
        if git_dir.parent()? != common.join("worktrees") {
            return None;
        }
        let back = real_path(&git_dir.join(read_trimmed(&git_dir.join("gitdir"))?));
        if back != real_path(root).join(".git") {
            return None;
        }
        if common.file_name()? != ".git" {
            // A bare repository's worktree: the repository itself, unless it
            // is somehow a checkout too.
            return (!common.join(".git").exists()).then_some(common);
        }
        common.parent().map(Path::to_path_buf)
    };
    resolve().unwrap_or_else(|| root.to_path_buf())
}

/// `p` with symlinks resolved, as the CLI's process would name it. On
/// Windows `canonicalize` returns a `\\?\` verbatim path, which names the
/// folder `----C--repo` where the CLI, seeing `C:\repo`, names it
/// `C--repo`. Tests that build a working directory take it from here too:
/// a raw `canonicalize` gives them a path no real cwd has (#3749).
pub fn real_path(p: &std::path::Path) -> std::path::PathBuf {
    without_verbatim_prefix(p.canonicalize().unwrap_or_else(|_| p.to_path_buf()))
}

/// `p` without a leading `\\?\`. A verbatim UNC path (`\\?\UNC\server\…`)
/// is kept: dropping only the prefix would leave `UNC\server\…`, which is
/// not the path the CLI sees.
fn without_verbatim_prefix(p: std::path::PathBuf) -> std::path::PathBuf {
    match p.to_str().and_then(|s| s.strip_prefix(r"\\?\")) {
        Some(rest) if !rest.starts_with("UNC\\") => std::path::PathBuf::from(rest),
        _ => p,
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    // Every expected value below was produced by the CLI's own `RC()`
    // (quoted on `project_dir_name`), run under Node — not by this module.

    #[test]
    fn a_windows_agent_directory() {
        assert_eq!(
            project_dir_name(r"C:\Users\user\.agentmux\agents\agent3-0630k"),
            "C--Users-user--agentmux-agents-agent3-0630k"
        );
    }

    #[test]
    fn a_posix_path_keeps_the_dashes_inside_a_segment() {
        assert_eq!(
            project_dir_name("/home/u/.agentmux/agents/foo-bar"),
            "-home-u--agentmux-agents-foo-bar"
        );
    }

    #[test]
    fn underscores_and_spaces_become_dashes() {
        assert_eq!(
            project_dir_name("/home/u/my_project dir"),
            "-home-u-my-project-dir"
        );
    }

    #[test]
    fn non_ascii_letters_and_punctuation_become_dashes() {
        assert_eq!(
            project_dir_name("C:\\code\\caf\u{e9} (old)@2~x"),
            "C--code-caf---old--2-x"
        );
    }

    #[test]
    fn a_character_outside_the_bmp_becomes_two_dashes() {
        assert_eq!(project_dir_name("/x/\u{1F600}y"), "-x---y");
    }

    #[test]
    fn a_long_path_is_cut_at_200_and_hashed() {
        let cwd = format!("/home/u/{}repo", "deeply_nested/".repeat(20));
        let got = project_dir_name(&cwd);
        assert_eq!(
            got,
            "-home-u-deeply-nested-deeply-nested-deeply-nested-deeply-nested-deeply-nested-\
             deeply-nested-deeply-nested-deeply-nested-deeply-nested-deeply-nested-deeply-nested-\
             deeply-nested-deeply-nested-deeply-nes-fygn57"
        );
        assert_eq!(got.len(), 207);
    }

    #[test]
    fn a_long_windows_path_is_cut_at_200_and_hashed() {
        let cwd = format!(r"C:\Users\user\{}", "a".repeat(230));
        assert_eq!(
            project_dir_name(&cwd),
            format!("C--Users-user-{}-eqzdy0", "a".repeat(186))
        );
    }

    #[test]
    fn exactly_200_characters_is_not_hashed() {
        let cwd = "a".repeat(200);
        assert_eq!(project_dir_name(&cwd), cwd);
    }

    #[test]
    fn the_hash_magnitude_of_i32_min_does_not_wrap() {
        // JavaScript's Math.abs(-2147483648) is 2147483648; an i32 `abs`
        // would overflow.
        assert_eq!(radix_36(i32::MIN.unsigned_abs()), "zik0zk");
        assert_eq!(radix_36(0), "0");
    }

    // The prefix rule is string handling, so it is checked on every
    // platform — PR CI runs no tests on Windows (ci-pr.yml).
    #[test]
    fn a_verbatim_drive_path_loses_its_prefix() {
        use std::path::PathBuf;
        assert_eq!(without_verbatim_prefix(PathBuf::from(r"\\?\C:\repo")), PathBuf::from(r"C:\repo"));
    }

    #[test]
    fn a_verbatim_unc_path_and_a_plain_path_are_kept() {
        use std::path::PathBuf;
        for p in [r"\\?\UNC\server\share\repo", r"C:\repo", "/home/u/repo"] {
            assert_eq!(without_verbatim_prefix(PathBuf::from(p)), PathBuf::from(p));
        }
    }

    /// A repository with one commit, and a linked worktree of it outside it,
    /// as the CLI's process would see them ([`real_path`]).
    pub(crate) fn repo_with_worktree(base: &std::path::Path) -> (std::path::PathBuf, std::path::PathBuf) {
        repo_and_worktree_at(&base.join("repo"), &base.join("wt"))
    }

    /// [`repo_with_worktree`], at the two folders named.
    pub(crate) fn repo_and_worktree_at(repo: &std::path::Path, wt: &std::path::Path) -> (std::path::PathBuf, std::path::PathBuf) {
        std::fs::create_dir_all(repo.join("sub")).unwrap();
        let git = |args: &[&str]| {
            use agentmux_common::win32::NoWindow;
            let mut cmd = std::process::Command::new("git");
            cmd.args(["-c", "user.email=a@b", "-c", "user.name=a"]).args(args).current_dir(repo);
            cmd.no_window();
            let out = cmd.output().unwrap();
            assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        };
        git(&["init", "-q"]);
        std::fs::write(repo.join("sub").join("x"), "x").unwrap();
        git(&["add", "."]);
        git(&["commit", "-qm", "x"]);
        git(&["worktree", "add", "-q", wt.to_str().unwrap()]);
        (real_path(repo), real_path(wt))
    }

    #[test]
    fn memory_is_keyed_by_the_main_checkout_for_subdirectories_and_worktrees() {
        let base = tempfile::tempdir().unwrap();
        let (repo, wt) = repo_with_worktree(base.path());
        assert_eq!(memory_project_root(&repo), repo);
        assert_eq!(memory_project_root(&repo.join("sub")), repo);
        assert_eq!(memory_project_root(&wt), repo, "a linked worktree shares the main checkout's memory");
        assert_eq!(memory_project_root(&wt.join("sub")), repo);
    }

    /// A folder a container agent can write is keyed by its own path, whatever it holds: a linked worktree
    /// (a `.git` file that points elsewhere) does not get the main checkout's memory, even one whose path
    /// differs from its own only by an ending, and a folder below a repository's top does not get the top's.
    #[test]
    fn a_folder_a_container_agent_can_write_is_keyed_by_its_own_path() {
        let base = tempfile::tempdir().unwrap();
        let (repo, wt) = repo_with_worktree(base.path());
        let none = |_: &std::path::Path| false;
        let the_worktree = |p: &std::path::Path| p.starts_with(&wt);
        assert_eq!(memory_project_root_with(&wt, &none), repo, "on the host a linked worktree shares the main checkout's");
        assert_eq!(memory_project_root_with(&wt, &the_worktree), wt);
        let below = |p: &std::path::Path| p.starts_with(repo.join("sub"));
        assert_eq!(memory_project_root_with(&repo.join("sub"), &below), repo.join("sub"), "nothing above it is looked at");

        // A workspace `w` that is a linked worktree of `w-b` and of nothing else: the folder of `w`, never of `w-b` or `w.b`.
        for ending in ["-b", ".b"] {
            let other = tempfile::tempdir().unwrap();
            let (main, w) = repo_and_worktree_at(&other.path().join(format!("w{ending}")), &other.path().join("w"));
            let name = |root: &std::path::Path| project_dir_name(&root.to_string_lossy());
            assert_eq!(memory_project_root_with(&w, &none), main, "on the host it shares the main checkout's");
            let got = memory_project_root_with(&w, &|p| p.starts_with(&w));
            assert_eq!(got, w);
            assert_ne!(name(&got), name(&main), "{ending}");
        }
    }

    /// What a container agent's workspace was keyed by before stays its key: a workspace that is no
    /// repository, and one that is a repository of its own, keep the folder they had.
    #[test]
    fn a_container_agents_workspace_keeps_the_folder_it_had() {
        let base = tempfile::tempdir().unwrap();
        let plain = real_path(base.path()).join("plain");
        std::fs::create_dir_all(&plain).unwrap();
        let (repo, _) = repo_with_worktree(base.path());
        for workspace in [plain, repo] {
            let before = memory_project_root_with(&workspace, &|_| false);
            let now = memory_project_root_with(&workspace, &|p| p.starts_with(&workspace));
            assert_eq!(now, before);
            assert_eq!(now, workspace);
        }
    }

    /// A worktree on the host whose repository a container agent can write keeps its own root.
    #[test]
    fn a_worktree_of_a_repository_a_container_agent_can_write_keeps_its_own_root() {
        let base = tempfile::tempdir().unwrap();
        let (repo, wt) = repo_with_worktree(base.path());
        assert_eq!(memory_project_root_with(&wt, &|p| p.starts_with(&repo)), wt);
    }

    /// Inside another folder a container agent can write, a link in the path is not followed.
    #[cfg(unix)]
    #[test]
    fn a_link_in_a_path_a_container_agent_can_write_is_not_followed() {
        let base = tempfile::tempdir().unwrap();
        let outer = real_path(base.path()).join("outer");
        let elsewhere = real_path(base.path()).join("elsewhere");
        std::fs::create_dir_all(&outer).unwrap();
        std::fs::create_dir_all(&elsewhere).unwrap();
        let inner = outer.join("inner");
        std::os::unix::fs::symlink(&elsewhere, &inner).unwrap();
        assert_eq!(memory_project_root_with(&inner, &|p| p.starts_with(&outer)), inner);
        assert_eq!(memory_project_root_with(&outer.join("x").join("..").join("inner"), &|p| p.starts_with(&outer)), inner);
        // The agent's own workspace, inside no other: its path is not the agent's to write, and resolves as before.
        let link = real_path(base.path()).join("link");
        std::os::unix::fs::symlink(&elsewhere, &link).unwrap();
        assert_eq!(memory_project_root_with(&link, &|p| p == link), elsewhere);
    }

    #[test]
    fn outside_a_repository_memory_is_keyed_by_the_physical_directory() {
        let base = tempfile::tempdir().unwrap();
        let real = real_path(base.path()).join("plain");
        std::fs::create_dir_all(&real).unwrap();
        assert_eq!(memory_project_root(&real), real);
        #[cfg(unix)]
        {
            let link = base.path().join("link");
            std::os::unix::fs::symlink(&real, &link).unwrap();
            assert_eq!(memory_project_root(&link), real);
        }
    }

    #[test]
    fn a_git_file_that_is_not_a_linked_worktree_keeps_its_own_root() {
        let base = tempfile::tempdir().unwrap();
        let dir = real_path(base.path()).join("odd");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(".git"), "gitdir: ../nowhere\n").unwrap();
        assert_eq!(memory_project_root(&dir), dir);
    }
}

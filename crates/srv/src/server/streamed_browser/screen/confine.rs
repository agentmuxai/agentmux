// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The shared-host form of the per-instance policy. Chrome reads its managed policy from a fixed folder and has no switch to change it, and a
//! file there would bind every Chrome of that kind on the machine (the streamed browser included). So a screen-mode browser is started inside
//! its own mount namespace (bubblewrap, an unprivileged user namespace), where only that folder is replaced by one the server wrote for this
//! instance. The same namespace hides the person's home folder from the browser: a local file the policy missed is not there to open.

use std::path::{Path, PathBuf};

/// The program that makes the namespace.
pub const PROGRAM: &str = "bwrap";

/// The nearest folder at or above `dir` that exists: where the replacement starts.
fn existing_base(dir: &Path) -> PathBuf {
    let mut base = dir;
    while !base.exists() {
        match base.parent() {
            Some(p) => base = p,
            None => break,
        }
    }
    base.to_path_buf()
}

/// The arguments for `bwrap`, up to and including the `--` before the browser's own command. `policy_src` is this instance's folder of policy
/// files; `policy_dir` is the folder the browser reads; `home` is hidden when it is a real folder.
pub fn args(policy_src: &Path, policy_dir: &Path, home: Option<&Path>) -> Vec<String> {
    let text = |p: &Path| p.display().to_string();
    let mut a: Vec<String> = ["--ro-bind", "/", "/", "--dev-bind", "/dev", "/dev", "--proc", "/proc", "--bind", "/tmp", "/tmp"].map(String::from).to_vec();
    if let Some(h) = home.filter(|h| h.is_absolute() && *h != Path::new("/")) {
        a.extend(["--tmpfs".to_string(), text(h)]);
    }
    let base = existing_base(policy_dir);
    // Never a memory folder over the root: if nothing above the policy folder exists, the bind below fails in words and nothing is hidden.
    if base != policy_dir && base.parent().is_some() {
        // The folder the browser reads is not there on this machine: make it (and what is above it up to the first one that exists) in memory.
        a.extend(["--tmpfs".to_string(), text(&base)]);
    }
    a.extend(["--bind".to_string(), text(policy_src), text(policy_dir)]);
    a.extend(["--cap-drop", "ALL", "--die-with-parent", "--new-session", "--"].map(String::from));
    a
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_policy_folder_is_replaced_and_the_home_folder_is_hidden() {
        let dir = tempfile::tempdir().unwrap();
        let (home, target) = (dir.path().join("home"), dir.path().join("managed"));
        std::fs::create_dir_all(&target).unwrap();
        let a = args(Path::new("/tmp/i/policy"), &target, Some(&home));
        let pair = |x: &str, y: &str| a.windows(2).any(|w| w[0] == x && w[1] == y);
        assert!(pair("--tmpfs", &home.display().to_string()), "{a:?}");
        assert!(a.windows(3).any(|w| w[0] == "--bind" && w[1] == "/tmp/i/policy" && w[2] == target.display().to_string()), "{a:?}");
        assert_eq!(a.last().map(String::as_str), Some("--"), "the browser's own command follows");
        assert!(a.contains(&"--cap-drop".to_string()) && a.contains(&"--die-with-parent".to_string()));
        assert!(!a.iter().any(|x| x == "--unshare-net"), "the network is the address proxy's business, not the namespace's");
    }

    #[test]
    fn a_missing_policy_folder_is_made_in_memory_from_the_first_folder_that_exists() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("etc").join("opt").join("policies").join("managed");
        std::fs::create_dir_all(dir.path().join("etc")).unwrap();
        let a = args(Path::new("/tmp/i/policy"), &target, None);
        let base = dir.path().join("etc");
        assert!(a.windows(2).any(|w| w[0] == "--tmpfs" && w[1] == base.display().to_string()), "{a:?}");
        // A home folder that is not a real place is not hidden: only the policy base is a memory folder.
        assert_eq!(args(Path::new("/p"), &target, Some(Path::new("/"))).iter().filter(|x| *x == "--tmpfs").count(), 1);
    }

    #[test]
    fn a_policy_folder_that_exists_is_bound_over_directly() {
        let dir = tempfile::tempdir().unwrap();
        let a = args(Path::new("/tmp/i/policy"), dir.path(), None);
        assert!(!a.contains(&"--tmpfs".to_string()), "{a:?}");
    }
}

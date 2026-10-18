// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Is a test binary running the code of the checkout that runs it?
//!
//! Cargo names a workspace's own crates the same in every clone of the
//! repository. Two clones that share one target directory (`CARGO_TARGET_DIR`)
//! therefore write the same artifacts, and cargo, which checks the source files
//! the existing artifact was built from, can call another clone's build fresh:
//! `cargo test` in one clone then runs the other clone's code and reports its
//! results as this clone's. Seen on 2026-10-17, with four clones sharing a
//! target dir: passes and failures that belonged to other branches, and a
//! "cannot find type" for code that was plainly there.
//!
//! Each crate has a test that calls [`check`]. The path compiled into the binary
//! (`env!("CARGO_MANIFEST_DIR")` at the call site) must be the crate folder of
//! the checkout `cargo test` runs in (its `CARGO_MANIFEST_DIR` at run time).

use std::path::Path;

/// `Err` with what to do when the test binary was compiled from another
/// checkout than the one running it. `compiled` is the caller's
/// `env!("CARGO_MANIFEST_DIR")`. `Ok` when the binary isn't run by cargo (no
/// `CARGO_MANIFEST_DIR` at run time), so it can't tell.
pub fn check(compiled: &str) -> Result<(), String> {
    match std::env::var("CARGO_MANIFEST_DIR") {
        Ok(running) if !running.is_empty() => compare(Path::new(compiled), Path::new(&running)),
        _ => Ok(()),
    }
}

fn compare(compiled: &Path, running: &Path) -> Result<(), String> {
    if same_folder(compiled, running) {
        return Ok(());
    }
    Err(format!(
        "this test binary was compiled from {} but cargo is running it for {}: the two checkouts share a \
         cargo target directory, so their builds overwrite each other and tests can run the other checkout's \
         code. Give each checkout its own target directory (unset CARGO_TARGET_DIR, or point it at a folder \
         of this checkout's own), then build again.",
        compiled.display(),
        running.display()
    ))
}

/// The same folder by path, after resolving links and `..`, or, when the paths
/// differ (a `subst` drive, a junction), because they hold the same
/// `Cargo.toml`: two clones' copies differ in their timestamps.
fn same_folder(a: &Path, b: &Path) -> bool {
    if let (Ok(ca), Ok(cb)) = (a.canonicalize(), b.canonicalize()) {
        if ca == cb {
            return true;
        }
    }
    let stamp = |p: &Path| {
        let m = std::fs::metadata(p.join("Cargo.toml")).ok()?;
        Some((m.len(), m.modified().ok()?, m.created().ok()))
    };
    matches!((stamp(a), stamp(b)), (Some(x), Some(y)) if x == y)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The guard itself, for this crate.
    #[test]
    fn this_test_binary_was_built_from_this_checkout() {
        check(env!("CARGO_MANIFEST_DIR")).unwrap();
    }

    /// The guard only works while cargo tells a test where it runs: under
    /// cargo (`CARGO` set), `CARGO_MANIFEST_DIR` must be there too, or every
    /// guard would pass without checking anything.
    #[test]
    fn cargo_says_which_checkout_runs_the_test() {
        if std::env::var_os("CARGO").is_some() {
            assert!(std::env::var_os("CARGO_MANIFEST_DIR").is_some(), "cargo ran this test without CARGO_MANIFEST_DIR");
        }
    }

    fn checkout(root: &Path) {
        std::fs::create_dir_all(root).unwrap();
        std::fs::write(root.join("Cargo.toml"), "[package]\n").unwrap();
    }

    #[test]
    fn another_checkouts_binary_is_refused_and_says_what_to_do() {
        let base = std::env::temp_dir().join(format!("agentmux-checkout-test-{}", std::process::id()));
        let (a, b) = (base.join("clone-a"), base.join("clone-b"));
        checkout(&a);
        std::thread::sleep(std::time::Duration::from_millis(20));
        checkout(&b);
        let err = compare(&a, &b).unwrap_err();
        assert!(err.contains("share a cargo target directory"), "{err}");
        assert!(err.contains("CARGO_TARGET_DIR"), "{err}");
        // The same folder by another spelling is the same checkout.
        assert!(compare(&a, &a.join("..").join("clone-a")).is_ok());
        let _ = std::fs::remove_dir_all(&base);
    }
}

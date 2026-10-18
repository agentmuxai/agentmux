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
//! Every test binary of the workspace (each crate's unit tests and each file
//! under `tests/`) invokes [`checkout_guard!`]. It runs [`guard`] when the
//! binary starts, before the test harness, so a test-name filter can't skip
//! it: the crate folder compiled into the binary (`CARGO_MANIFEST_DIR` at build
//! time) must be the one cargo runs it for (`CARGO_MANIFEST_DIR` at run time).

use std::path::Path;

/// `Err` with what to do when the test binary was compiled from another
/// checkout than the one running it. `compiled` is the invoking crate's
/// `env!("CARGO_MANIFEST_DIR")`. `Ok` when the binary isn't run by cargo (no
/// `CARGO_MANIFEST_DIR` at run time), so it can't tell.
pub fn check(compiled: &str) -> Result<(), String> {
    match std::env::var("CARGO_MANIFEST_DIR") {
        Ok(running) if !running.is_empty() => compare(Path::new(compiled), Path::new(&running)),
        _ => Ok(()),
    }
}

/// What [`checkout_guard!`] runs at a test binary's start: on a mismatch, the
/// reason on stderr and exit code 101, as a failed test run would.
#[doc(hidden)]
pub fn guard(compiled: &str) {
    if let Err(e) = check(compiled) {
        eprintln!("error: {e}");
        std::process::exit(101);
    }
}

/// Runs [`guard`] for the invoking crate when its test binary starts, from the
/// platform's initializer section (as the `ctor` crate does). Invoke it once
/// per test binary: at a crate's root, behind `#[cfg(test)]` where the crate
/// has `agentmux-common` only as a dev-dependency, and at the top of each
/// file under `tests/`.
#[macro_export]
macro_rules! checkout_guard {
    () => {
        const _: () = {
            extern "C" fn agentmux_checkout_guard() {
                $crate::checkout::guard(env!("CARGO_MANIFEST_DIR"));
            }
            #[used]
            #[cfg_attr(any(target_os = "linux", target_os = "android", target_os = "freebsd"), link_section = ".init_array")]
            #[cfg_attr(target_os = "windows", link_section = ".CRT$XCU")]
            #[cfg_attr(target_vendor = "apple", link_section = "__DATA,__mod_init_func")]
            static AGENTMUX_CHECKOUT_GUARD: extern "C" fn() = agentmux_checkout_guard;
        };
    };
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

/// The same folder: the same path once links and `..` are resolved, or, when
/// the paths differ (a `subst` drive, a junction), the same `Cargo.toml` file
/// on disk, by file identity, never by look-alike metadata.
fn same_folder(a: &Path, b: &Path) -> bool {
    if let (Ok(ca), Ok(cb)) = (a.canonicalize(), b.canonicalize()) {
        if ca == cb {
            return true;
        }
    }
    same_file::is_same_file(a.join("Cargo.toml"), b.join("Cargo.toml")).unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

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
        checkout(&b);
        let err = compare(&a, &b).unwrap_err();
        assert!(err.contains("share a cargo target directory"), "{err}");
        assert!(err.contains("CARGO_TARGET_DIR"), "{err}");
        // Two copies with the same contents and timestamps are still two.
        let stamp = std::fs::metadata(a.join("Cargo.toml")).unwrap().modified().unwrap();
        std::fs::File::options().write(true).open(b.join("Cargo.toml")).unwrap().set_modified(stamp).unwrap();
        assert!(compare(&a, &b).is_err(), "look-alike copies aren't the same checkout");
        // The same folder by another spelling is the same checkout.
        assert!(compare(&a, &a.join("..").join("clone-a")).is_ok());
        let _ = std::fs::remove_dir_all(&base);
    }
}

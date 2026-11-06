// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::*;
use std::sync::atomic::AtomicBool;

fn quiet(_: &'static str, _: &str) {}

/// Requirement lines of a lock: each entry's text with its `--hash` continuation lines.
fn entries(lock: &str) -> Vec<(String, Vec<String>)> {
    let mut out: Vec<(String, Vec<String>)> = Vec::new();
    for raw in lock.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(h) = line.trim_end_matches('\\').trim().strip_prefix("--hash=") {
            out.last_mut().expect("a hash before any requirement").1.push(h.to_string());
        } else {
            out.push((line.trim_end_matches('\\').trim().to_string(), Vec::new()));
        }
    }
    out
}

#[test]
fn every_uv_install_names_a_registered_provider_its_pin_and_a_tight_lock() {
    for spec in UV_SPECS {
        let p = crate::backend::providers::get_provider(spec.provider_id).expect(spec.provider_id);
        assert_eq!(p.pinned_version, spec.version, "{}: the spec and the provider disagree on the pin", spec.provider_id);
        assert!(p.npm_package.is_empty(), "{}: a uv provider has no npm package", spec.provider_id);
        assert!(crate::backend::cli_download::download_spec(spec.provider_id).is_none(), "{}: one install kind per provider", spec.provider_id);
        assert!(spec.python.split('.').count() == 2 && spec.python.starts_with("3."), "{}", spec.python);

        let reqs = entries(spec.lock);
        assert!(reqs.len() > 20, "{}: a lock holds the whole tree", spec.provider_id);
        let top = format!("{}=={}", spec.package, spec.version);
        assert!(reqs.iter().any(|(r, _)| r.starts_with(&top)), "{}: the lock does not pin {top}", spec.provider_id);
        for (req, hashes) in &reqs {
            assert!(req.contains("=="), "{}: {req:?} is not an exact pin", spec.provider_id);
            assert!(!hashes.is_empty(), "{}: {req:?} has no hash", spec.provider_id);
            for h in hashes {
                let hex = h.strip_prefix("sha256:").unwrap_or("");
                assert!(hex.len() == 64 && hex.chars().all(|c| c.is_ascii_hexdigit()), "{}: bad hash on {req:?}", spec.provider_id);
            }
            assert!(!req.contains("://") && !req.contains(" @ ") && !req.starts_with('-'), "{}: {req:?} names a source", spec.provider_id);
        }
        for pkg in spec.build_from_sdist {
            let prefix = format!("{pkg}==");
            assert!(reqs.iter().any(|(r, _)| r.starts_with(&prefix)), "{pkg} is built from source but not in the lock");
            assert!(reqs.iter().any(|(r, _)| r.starts_with("setuptools==")), "an sdist build needs setuptools in the lock");
        }
    }
}

#[test]
fn the_build_tools_are_taken_out_of_the_lock_with_their_hashes() {
    for spec in UV_SPECS.iter().filter(|s| !s.build_from_sdist.is_empty()) {
        let got = entries(&lock_entries(spec.lock, BUILD_TOOLS));
        let names: Vec<&str> = got.iter().map(|(r, _)| r.split("==").next().unwrap()).collect();
        assert_eq!(names, ["setuptools", "wheel"], "{}", spec.provider_id);
        assert!(got.iter().all(|(_, h)| !h.is_empty()), "{}: a build tool lost its hashes", spec.provider_id);
    }
    let text = "# c\na==1 \\\n    --hash=sha256:x \\\n    # via y\nb==2 \\\n    --hash=sha256:y\n";
    assert_eq!(lock_entries(text, &["b"]), "b==2 \\\n    --hash=sha256:y\n");
    assert_eq!(lock_entries(text, &["c"]), "");
}

#[test]
fn only_a_registered_provider_has_a_uv_install() {
    assert!(uv_spec("aider").is_some() && uv_spec("kimi").is_some());
    for id in ["goose", "claude", "../aider", "", "aider "] {
        assert!(uv_spec(id).is_none(), "{id:?}");
    }
    assert!(crate::backend::cli_download::is_managed_install("aider"));
    assert!(crate::backend::cli_download::is_managed_install("kimi"));
    assert!(crate::backend::cli_download::is_managed_install("goose"));
    assert!(!crate::backend::cli_download::is_managed_install("claude"));
}

#[test]
fn the_pinned_uv_has_a_well_formed_asset_per_platform_and_is_not_a_provider() {
    assert!(crate::backend::providers::get_provider("uv").is_none());
    assert!(UV.url_prefix.starts_with("https://") && UV.url_prefix.contains(&format!("/{}/", UV.version)));
    let mut seen = std::collections::HashSet::new();
    for a in UV.assets {
        assert!(seen.insert((a.os, a.arch)), "duplicate {} {}", a.os, a.arch);
        assert!(a.url.starts_with(UV.url_prefix), "{}", a.url);
        assert_eq!(a.sha256.len(), 64, "{}", a.url);
        assert!(a.sha256.chars().all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c)), "{}", a.url);
        assert!(a.size > 1_000_000, "{}", a.url);
        match a.archive {
            Archive::Zip => assert_eq!(a.exe_in_archive, "uv.exe"),
            Archive::Raw => panic!("uv ships in an archive: {}", a.url),
            Archive::TarGz => assert!(a.exe_in_archive.ends_with("/uv") && !a.exe_in_archive.contains(".."), "{}", a.exe_in_archive),
        }
    }
    for (os, arch) in [("windows", "x86_64"), ("windows", "aarch64"), ("macos", "aarch64"), ("macos", "x86_64"), ("linux", "x86_64"), ("linux", "aarch64")] {
        assert!(UV.asset_for(os, arch).is_some(), "no uv for {os} {arch}");
    }
}

#[test]
fn the_program_is_in_the_venv_scripts_on_windows_and_bin_elsewhere() {
    let bin = venv_bin(Path::new("d"), "aider");
    let parent = bin.parent().unwrap();
    assert_eq!(parent, Path::new("d").join("venv").join(if cfg!(windows) { "Scripts" } else { "bin" }));
    assert_eq!(bin.file_name().unwrap().to_string_lossy(), if cfg!(windows) { "aider.exe" } else { "aider" });
}

#[test]
fn an_install_dir_with_a_venv_program_counts_only_once_marked_complete() {
    let dir = tempfile::tempdir().unwrap();
    let bin = venv_bin(dir.path(), "aider");
    std::fs::create_dir_all(bin.parent().unwrap()).unwrap();
    std::fs::write(&bin, b"").unwrap();
    assert_eq!(cli_install::installed_bin(dir.path(), "aider"), Some(bin));
    assert!(!cli_install::is_valid_install(dir.path(), "aider"), "no marker, no install");
    cli_install::mark_complete(dir.path(), "0.86.2").unwrap();
    assert!(cli_install::is_valid_install(dir.path(), "aider"));
}

#[test]
fn the_lock_beside_a_folder_is_held_until_dropped() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("tools").join("uv").join("0.13.7");
    let first = cli_install::lock_dir(&dir).unwrap();
    assert!(tmp.path().join("tools").join("uv").join("0.13.7.lock").is_file());
    let (tx, rx) = std::sync::mpsc::channel();
    let d2 = dir.clone();
    let t = std::thread::spawn(move || {
        let _second = cli_install::lock_dir(&d2).unwrap();
        tx.send(()).unwrap();
    });
    assert!(rx.recv_timeout(Duration::from_millis(400)).is_err(), "a second instance waits while the first holds the lock");
    drop(first);
    rx.recv_timeout(Duration::from_secs(10)).expect("the second instance gets the lock once it is released");
    t.join().unwrap();
}

/// A stand-in for uv: prints two lines, then exits with `code`.
fn fake_uv(dir: &Path, code: i32, sleep_secs: u32) -> PathBuf {
    if cfg!(windows) {
        let p = dir.join("uv.cmd");
        let wait = if sleep_secs > 0 { format!("ping -n {} 127.0.0.1 >nul\r\n", sleep_secs + 1) } else { String::new() };
        std::fs::write(&p, format!("@echo off\r\necho resolving\r\necho boom 1>&2\r\n{wait}exit /b {code}\r\n")).unwrap();
        p
    } else {
        let p = dir.join("uv");
        let wait = if sleep_secs > 0 { format!("sleep {sleep_secs}\n") } else { String::new() };
        std::fs::write(&p, format!("#!/bin/sh\necho resolving\necho boom 1>&2\n{wait}exit {code}\n")).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        p
    }
}

#[tokio::test]
async fn a_failing_uv_step_reports_its_exit_code_and_its_last_output() {
    let tmp = tempfile::tempdir().unwrap();
    let uv = fake_uv(tmp.path(), 3, 0);
    let seen = parking_lot::Mutex::new(Vec::<String>::new());
    let on_line = |_: &'static str, l: &str| seen.lock().push(l.to_string());
    let stop = AtomicBool::new(false);
    let run = UvRun { uv: &uv, dir: tmp.path(), on_line: &on_line, cancel: &stop };
    let err = run.run(&["pip", "install"]).await.unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("uv exited 3") && msg.contains("boom"), "{msg}");
    let seen = seen.lock();
    assert!(seen.iter().any(|l| l == "resolving") && seen.iter().any(|l| l == "boom"), "{seen:?}");
}

#[tokio::test]
async fn a_uv_step_that_succeeds_is_ok() {
    let tmp = tempfile::tempdir().unwrap();
    let uv = fake_uv(tmp.path(), 0, 0);
    let stop = AtomicBool::new(false);
    let run = UvRun { uv: &uv, dir: tmp.path(), on_line: &quiet, cancel: &stop };
    run.run(&["venv"]).await.expect("exit 0");
}

#[tokio::test]
async fn cancelling_stops_a_running_uv_step() {
    let tmp = tempfile::tempdir().unwrap();
    let uv = fake_uv(tmp.path(), 0, 30);
    let stop = AtomicBool::new(false);
    let run = UvRun { uv: &uv, dir: tmp.path(), on_line: &quiet, cancel: &stop };
    let started = std::time::Instant::now();
    let (res, _) = tokio::join!(run.run(&["pip", "install"]), async {
        tokio::time::sleep(Duration::from_millis(600)).await;
        stop.store(true, Ordering::Relaxed);
    });
    assert!(matches!(res, Err(DownloadError::Cancelled)), "{res:?}");
    assert!(started.elapsed() < Duration::from_secs(15), "cancel did not wait out the step");
}

#[tokio::test]
async fn a_uv_that_cannot_start_says_so() {
    let tmp = tempfile::tempdir().unwrap();
    let stop = AtomicBool::new(false);
    let missing = tmp.path().join("no-such-uv");
    let run = UvRun { uv: &missing, dir: tmp.path(), on_line: &quiet, cancel: &stop };
    let err = run.run(&["venv"]).await.unwrap_err();
    assert!(err.to_string().contains("cannot run uv"), "{err}");
}

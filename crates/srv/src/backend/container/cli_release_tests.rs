// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::*;
use crate::backend::container::cli::CliInstall;
use crate::backend::providers;

const NON_NPM: [(&str, Source); 6] = [
    ("goose", Source::Download),
    ("grok", Source::Download),
    ("muse", Source::Download),
    ("antigravity", Source::Download),
    ("aider", Source::Uv),
    ("kimi", Source::Uv),
];

fn install(id: &str) -> CliInstall {
    let provider = providers::get_provider(id).unwrap_or_else(|| panic!("{id} is a provider"));
    CliInstall::for_provider(id, provider.cli_command).unwrap_or_else(|| panic!("{id} has a container route"))
}

#[test]
fn the_six_that_are_not_on_npm_each_have_a_container_route_at_the_pin_the_host_installs() {
    for (id, source) in NON_NPM {
        let i = install(id);
        assert_eq!(i.source, source, "{id}");
        let provider = providers::get_provider(id).unwrap();
        assert_eq!(i.version, provider.pinned_version, "{id}: the container installs the pin the registry holds");
        assert_eq!(i.specs, vec![format!("{id}@{}", provider.pinned_version)], "{id}");
        // The marker is a plain file name that changes with the pin.
        let marker = i.marker();
        assert!(marker.starts_with(".installed-") && !marker.contains('/'), "{marker}");
    }
}

#[test]
fn a_custom_command_or_an_unknown_provider_has_no_route() {
    assert_eq!(CliInstall::for_provider("goose", "my-goose"), None);
    assert_eq!(CliInstall::for_provider("no-such-provider", "x"), None);
}

#[test]
fn npm_providers_keep_the_npm_route() {
    let i = install("claude");
    assert_eq!(i.source, Source::Npm);
    assert!(i.uploads().is_empty());
}

#[test]
fn every_registry_asset_set_has_a_linux_build_for_both_machines() {
    for spec in cli_download::SPECS {
        assert!(has_linux(spec.assets), "{} has no x86_64 and aarch64 Linux build", spec.provider_id);
    }
    assert!(has_linux(cli_uv::UV.assets), "uv has no x86_64 and aarch64 Linux build");
}

#[test]
fn the_asset_groups_carry_the_registry_values_six_at_a_time() {
    for spec in cli_download::SPECS {
        let params = asset_params(spec.assets);
        assert_eq!(params.len() % 6, 0, "{}", spec.provider_id);
        assert_eq!(params.len() / 6, spec.assets.iter().filter(|a| a.os == "linux").count());
        for g in params.chunks(6) {
            assert!(g[0] == "x86_64" || g[0] == "aarch64", "{g:?}");
            assert!(g[1].starts_with("https://") && g[1].contains(spec.version), "{}: {}", spec.provider_id, g[1]);
            assert_eq!(g[2].len(), 64, "{}: a SHA-256 is 64 hex digits", spec.provider_id);
            assert!(g[3].parse::<u64>().is_ok_and(|n| n > 0), "{g:?}");
            assert!(g[4] == "tar.gz" || g[4] == "raw", "{}: a Linux build is a tarball or the program itself: {g:?}", spec.provider_id);
        }
    }
}

#[test]
fn the_download_argv_hands_the_registry_values_over_as_parameters_never_in_the_script() {
    let i = install("goose");
    let argv = i.install_argv(i.limits());
    assert_eq!(&argv[0..2], ["sh", "-c"]);
    assert_eq!(argv[3], "agentmux-cli-install");
    assert_eq!(argv[4], "/home/agent/.agentmux/cli");
    assert_eq!(argv[5], i.marker());
    assert_eq!(argv[6], "goose");
    let spec = cli_download::download_spec("goose").unwrap();
    for a in spec.assets.iter().filter(|a| a.os == "linux") {
        assert!(argv.contains(&a.url.to_string()) && argv.contains(&a.sha256.to_string()), "{}", a.url);
        assert!(!argv[2].contains(a.sha256) && !argv[2].contains(a.url), "the script text carries none of the registry's values");
    }
    // No Windows or macOS asset is offered to a Linux container.
    assert!(!argv.iter().any(|p| p.contains("windows") || p.contains("apple-darwin")), "{argv:?}");
}

#[test]
fn the_uv_argv_names_the_python_the_lock_files_and_the_pinned_uv() {
    for id in ["aider", "kimi"] {
        let i = install(id);
        assert_eq!(i.source, Source::Uv);
        let argv = i.install_argv(i.limits());
        let spec = cli_uv::uv_spec(id).unwrap();
        assert_eq!(argv[8], spec.python);
        assert_eq!(argv[9], format!("/tmp/agentmux-lock-{id}.txt"));
        assert_eq!(argv[10], format!("/tmp/agentmux-tools-{id}.txt"));
        assert_eq!(argv[11], spec.build_from_sdist.join(" "));
        for a in cli_uv::UV.assets.iter().filter(|a| a.os == "linux") {
            assert!(argv.contains(&a.url.to_string()), "{}", a.url);
        }
        // The lock is a file, not an argument: it is larger than one argument may be.
        assert!(spec.lock.len() > 128 * 1024, "{id}: the lock does not fit an argument, which is why it is uploaded");
        assert!(argv.iter().all(|p| p.len() < 64 * 1024));
        let up = i.uploads();
        assert_eq!(up.len(), 2);
        assert_eq!(up[0].name, format!("agentmux-lock-{id}.txt"));
        assert_eq!(up[0].bytes, spec.lock.as_bytes());
        assert_eq!(up[1].bytes, cli_uv::build_tools_lock(spec).into_bytes());
    }
}

#[test]
fn a_python_install_gets_more_time_than_a_download() {
    assert!(install("aider").limits().run > install("goose").limits().run);
    assert!(install("aider").limits().script > install("goose").limits().script);
    // The host-side backstop is longer than the script's own limit, or it would cut the script off first.
    for (id, _) in NON_NPM {
        let l = install(id).limits();
        assert!(l.run > l.script, "{id}");
    }
}

#[test]
fn the_scripts_fetch_over_https_only_and_check_the_hash_before_anything_is_installed() {
    let script = DOWNLOAD_SCRIPT.as_str();
    assert!(script.contains("--proto '=https'"));
    assert!(script.contains("sha256sum"));
    let fetch = script.find("fetch \"$URL\"").expect("fetches");
    let mv = script.find("mv \"$STAGE/bin\"").expect("swaps in");
    let marker = script.rfind(": > \"$DIR/$MARKER\"").expect("writes the marker");
    assert!(fetch < mv && mv < marker, "fetch, then swap, then the marker last");
    assert!(UV_SCRIPT.contains("--require-hashes") && UV_SCRIPT.contains("--only-binary :all:"));
    // Never a vendor script.
    assert!(!script.contains("| sh") && !script.contains("| bash") && !UV_SCRIPT.contains("| sh"));
}

/// Runs the real download script in a shell with a fake `curl` that serves a local file.
#[cfg(unix)]
pub mod shell {
    use super::*;
    use sha2::{Digest, Sha256};
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    fn machine() -> Option<&'static str> {
        match std::env::consts::ARCH {
            "x86_64" => Some("x86_64"),
            "aarch64" => Some("aarch64"),
            _ => None,
        }
    }

    fn sha(path: &Path) -> String {
        let bytes = std::fs::read(path).unwrap();
        Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect()
    }

    struct Rig {
        dir: PathBuf,
        path: String,
    }

    impl Rig {
        /// None when this host cannot run the container scripts (they need coreutils `timeout`, `sha256sum`).
        fn new(name: &str) -> Option<Self> {
            machine()?;
            let ok = |c: &str| Command::new("sh").args(["-c", &format!("command -v {c}")]).output().is_ok_and(|o| o.status.success());
            if !ok("timeout") || !ok("sha256sum") || !ok("tar") {
                eprintln!("skipped: the script targets the Linux agent container (needs timeout, sha256sum, tar)");
                return None;
            }
            let dir = std::env::temp_dir().join(format!("agentmux-cli-release-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(dir.join("fakebin")).unwrap();
            // A curl that "downloads" $FIXTURE, whatever the URL.
            let curl = dir.join("fakebin/curl");
            std::fs::write(&curl, "#!/bin/sh\nwhile [ \"$1\" != \"-o\" ]; do shift; done\ncp \"$FIXTURE\" \"$2\"\n").unwrap();
            std::fs::set_permissions(&curl, std::fs::Permissions::from_mode(0o755)).unwrap();
            let path = format!("{}:{}", dir.join("fakebin").display(), std::env::var("PATH").unwrap_or_default());
            Some(Self { dir, path })
        }

        fn run(&self, fixture: &Path, sha: &str, size: u64, kind: &str, exe: &str, marker: &str) -> bool {
            let cli = self.dir.join("cli");
            let out = Command::new("sh")
                .args(["-c", DOWNLOAD_SCRIPT.as_str(), "agentmux-cli-install"])
                .arg(&cli)
                .args([marker, "fakecli", "60"])
                .args([machine().unwrap(), "https://example.invalid/fakecli", sha, &size.to_string(), kind, exe])
                .env("PATH", &self.path)
                .env("FIXTURE", fixture)
                .output()
                .unwrap();
            if !out.status.success() {
                eprintln!("{}", String::from_utf8_lossy(&out.stderr));
            }
            out.status.success()
        }

        fn cli(&self) -> PathBuf {
            self.dir.join("cli")
        }
    }

    impl Drop for Rig {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    fn write(path: &Path, text: &str) {
        std::fs::write(path, text).unwrap();
    }

    #[test]
    fn a_raw_download_with_the_pinned_hash_is_installed_executable_and_marked_last() {
        let Some(rig) = Rig::new("raw") else { return };
        let file = rig.dir.join("fixture");
        write(&file, "#!/bin/sh\necho 1.0\n");
        assert!(rig.run(&file, &sha(&file), 20, "raw", "", ".installed-fakecli-1"));
        let bin = rig.cli().join("bin/fakecli");
        assert!(bin.is_file());
        assert_ne!(std::fs::metadata(&bin).unwrap().permissions().mode() & 0o111, 0, "executable");
        assert!(rig.cli().join(".installed-fakecli-1").is_file());
        assert!(!rig.cli().join(".staging").exists());
        // A new pin replaces the old install and its marker.
        write(&file, "#!/bin/sh\necho 2.0\n");
        assert!(rig.run(&file, &sha(&file), 20, "raw", "", ".installed-fakecli-2"));
        assert!(!rig.cli().join(".installed-fakecli-1").exists());
        assert!(rig.cli().join(".installed-fakecli-2").is_file());
    }

    #[test]
    fn a_file_with_another_hash_is_refused_and_leaves_nothing_behind() {
        let Some(rig) = Rig::new("hash") else { return };
        let file = rig.dir.join("fixture");
        write(&file, "#!/bin/sh\necho 1.0\n");
        assert!(!rig.run(&file, &"0".repeat(64), 20, "raw", "", ".installed-fakecli-1"));
        assert!(!rig.cli().join("bin").exists());
        assert!(!rig.cli().join(".installed-fakecli-1").exists());
        assert!(!rig.cli().join(".staging").exists(), "staging is cleaned up on failure");
    }

    #[test]
    fn a_file_larger_than_the_pinned_size_plus_a_megabyte_is_refused() {
        let Some(rig) = Rig::new("size") else { return };
        let file = rig.dir.join("fixture");
        std::fs::write(&file, vec![b'x'; 2 * 1024 * 1024]).unwrap();
        assert!(!rig.run(&file, &sha(&file), 10, "raw", "", ".installed-fakecli-1"));
        assert!(!rig.cli().join(".installed-fakecli-1").exists());
    }

    #[test]
    fn a_tarball_gives_up_only_the_named_plain_file() {
        let Some(rig) = Rig::new("tar") else { return };
        let src = rig.dir.join("src");
        std::fs::create_dir_all(src.join("pkg")).unwrap();
        write(&src.join("pkg/fakecli"), "#!/bin/sh\necho 1.0\n");
        write(&src.join("pkg/other"), "not wanted");
        let tgz = rig.dir.join("fixture.tgz");
        assert!(Command::new("tar").args(["-czf"]).arg(&tgz).arg("-C").arg(&src).arg("pkg").status().unwrap().success());
        let size = std::fs::metadata(&tgz).unwrap().len();
        assert!(rig.run(&tgz, &sha(&tgz), size, "tar.gz", "pkg/fakecli", ".installed-fakecli-1"));
        assert!(rig.cli().join("bin/fakecli").is_file());
        assert!(!rig.cli().join("bin/other").exists(), "nothing else of the archive is installed");
        // A name the archive does not have fails, and installs nothing new.
        assert!(!rig.run(&tgz, &sha(&tgz), size, "tar.gz", "pkg/missing", ".installed-fakecli-2"));
        assert!(!rig.cli().join(".installed-fakecli-2").exists());
    }

    #[test]
    fn a_tarball_whose_member_names_a_parent_folder_installs_nothing() {
        let Some(rig) = Rig::new("dotdot") else { return };
        let src = rig.dir.join("src");
        std::fs::create_dir_all(&src).unwrap();
        write(&src.join("fakecli"), "#!/bin/sh\necho 1.0\n");
        let tgz = rig.dir.join("fixture.tgz");
        // Members are named `../fakecli`.
        let made = Command::new("tar")
            .args(["-czf"])
            .arg(&tgz)
            .arg("-C")
            .arg(&src)
            .args(["--transform", "s,^,../,", "fakecli"])
            .status();
        if !made.is_ok_and(|s| s.success()) {
            eprintln!("skipped: this tar cannot make a `..` member");
            return;
        }
        let size = std::fs::metadata(&tgz).unwrap().len();
        assert!(!rig.run(&tgz, &sha(&tgz), size, "tar.gz", "fakecli", ".installed-fakecli-1"));
        assert!(!rig.cli().join(".installed-fakecli-1").exists());
        assert!(!rig.dir.join("fakecli").exists(), "nothing was written above the staging folder");
    }

    #[test]
    fn a_machine_without_a_build_fails_before_downloading_anything() {
        let Some(rig) = Rig::new("nomachine") else { return };
        let file = rig.dir.join("fixture");
        write(&file, "x");
        let other = if machine() == Some("x86_64") { "aarch64" } else { "x86_64" };
        let out = Command::new("sh")
            .args(["-c", DOWNLOAD_SCRIPT.as_str(), "agentmux-cli-install"])
            .arg(rig.cli())
            .args([".installed-x", "fakecli", "60", other, "https://example.invalid/x", &sha(&file), "1", "raw", ""])
            .env("PATH", &rig.path)
            .env("FIXTURE", &file)
            .output()
            .unwrap();
        assert!(!out.status.success());
        assert!(String::from_utf8_lossy(&out.stderr).contains("there is no build of fakecli"), "{}", String::from_utf8_lossy(&out.stderr));
    }
}

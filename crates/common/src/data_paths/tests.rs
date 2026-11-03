// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::dir_links::ensure_owner_only_dir_inner;
use super::*;
use crate::TEST_ENV_LOCK;
use tempfile::TempDir;

/// RAII guard that restores process state on drop, even if the
/// test panics. Without Drop-based cleanup, a panic inside `f`
/// would leave AGENTMUX_HOME_OVERRIDE set with a stale tempdir
/// path AND poison the mutex; subsequent tests recover from poison
/// but inherit the wrong env value.
struct HomeOverrideGuard {
    _tmp: TempDir,
    _lock: std::sync::MutexGuard<'static, ()>,
}

impl Drop for HomeOverrideGuard {
    fn drop(&mut self) {
        std::env::remove_var("AGENTMUX_HOME_OVERRIDE");
    }
}

fn with_home_override<F: FnOnce(&Path)>(f: F) {
    let lock = TEST_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let tmp = TempDir::new().expect("tempdir");
    let path = tmp.path().to_path_buf();
    std::env::set_var("AGENTMUX_HOME_OVERRIDE", &path);
    let _guard = HomeOverrideGuard { _tmp: tmp, _lock: lock };
    f(&path);
    // _guard drops here, removing the env var even if f panicked
    // (the panic still propagates after Drop runs).
}

/// Helper: clear AGENTMUX_CHANNEL inside an existing
/// with_home_override block to test pure mode-default resolution.
/// Channel resolution reads the live env var, so individual tests
/// must clear it to avoid leakage from sibling tests running
/// concurrently inside the same process (TEST_ENV_LOCK serializes
/// HOME_OVERRIDE but the channel var is a separate axis).
fn clear_channel_env() {
    std::env::remove_var("AGENTMUX_CHANNEL");
}

/// A10 compatibility guarantee. srv's old `get_mux_data_dir` read
/// AGENTMUX_DATA_HOME; this module's resolver read AGENTMUX_HOME_OVERRIDE.
/// Consolidating onto one resolver must not move where a live install
/// finds its data, so BOTH are honoured — and an install that only ever
/// set AGENTMUX_DATA_HOME must still resolve to exactly that path.
#[test]
fn agentmux_root_honours_srvs_env_var_for_existing_installs() {
    let _lock = TEST_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    std::env::remove_var("AGENTMUX_HOME_OVERRIDE");
    let tmp = TempDir::new().expect("tempdir");
    std::env::set_var("AGENTMUX_DATA_HOME", tmp.path());
    let got = agentmux_root().expect("root resolves");
    std::env::remove_var("AGENTMUX_DATA_HOME");
    assert_eq!(got, tmp.path(), "AGENTMUX_DATA_HOME must still win");
}

/// With neither var set the answer is `~/.agentmux` — unchanged from both
/// pre-consolidation resolvers, which is the case every default install is
/// in. Asserted against dirs::home_dir() rather than a literal so this
/// fails if the default layout ever moves.
#[test]
fn agentmux_root_defaults_to_home_dot_agentmux() {
    let _lock = TEST_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    std::env::remove_var("AGENTMUX_HOME_OVERRIDE");
    std::env::remove_var("AGENTMUX_DATA_HOME");
    let got = agentmux_root().expect("root resolves");
    let want = dirs::home_dir().expect("home").join(".agentmux");
    assert_eq!(got, want);
}

/// Precedence is documented, not accidental: HOME_OVERRIDE is this
/// module's test hook and wins, so a test that sets it is not silently
/// overridden by an AGENTMUX_DATA_HOME leaked from the ambient
/// environment.
#[test]
fn agentmux_root_prefers_home_override_over_data_home() {
    let _lock = TEST_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let a = TempDir::new().expect("tempdir");
    let b = TempDir::new().expect("tempdir");
    std::env::set_var("AGENTMUX_HOME_OVERRIDE", a.path());
    std::env::set_var("AGENTMUX_DATA_HOME", b.path());
    let got = agentmux_root().expect("root resolves");
    std::env::remove_var("AGENTMUX_HOME_OVERRIDE");
    std::env::remove_var("AGENTMUX_DATA_HOME");
    assert_eq!(got, a.path());
}

/// An empty value is not a path. Both resolvers skipped empty strings
/// before; the consolidated one must too, or `AGENTMUX_DATA_HOME=```
/// in a shell profile would redirect every install to "".
#[test]
fn agentmux_root_ignores_empty_env_values() {
    let _lock = TEST_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    std::env::set_var("AGENTMUX_HOME_OVERRIDE", "");
    std::env::set_var("AGENTMUX_DATA_HOME", "");
    let got = agentmux_root().expect("root resolves");
    std::env::remove_var("AGENTMUX_HOME_OVERRIDE");
    std::env::remove_var("AGENTMUX_DATA_HOME");
    assert_eq!(got, dirs::home_dir().expect("home").join(".agentmux"));
}

#[test]
fn installed_paths_under_default_channel() {
    with_home_override(|root| {
        clear_channel_env();
        let ver = "0.41.0";
        let p = DataPaths::resolve(ver, &RuntimeMode::Installed).unwrap();
        // Channel-level root (instance_dir).
        assert_eq!(p.channel, "stable");
        let ch = root.join("channels").join("stable");
        assert_eq!(p.instance_dir, ch);
        // Version-scoped dirs live under versions/<ver>/.
        let vd = ch.join("versions").join(ver);
        assert_eq!(p.data_dir,             vd.join("data"));
        assert_eq!(p.logs_dir,             vd.join("logs"));
        assert_eq!(p.cef_cache_dir,        vd.join("cef-cache"));
        assert_eq!(p.instance_runtime_dir, vd.join("runtime"));
        // Channel-wide dirs stay at instance_dir level.
        assert_eq!(p.config_dir,  ch.join("config"));
        assert_eq!(p.agents_dir,  ch.join("agents"));
        assert_eq!(p.shared_dir,  root.join("shared"));
    });
}

#[test]
fn two_installed_versions_have_distinct_data_dirs() {
    with_home_override(|root| {
        clear_channel_env();
        let p1 = DataPaths::resolve("0.40.2", &RuntimeMode::Installed).unwrap();
        let p2 = DataPaths::resolve("0.41.0", &RuntimeMode::Installed).unwrap();
        // Same channel root — agents and config are shared.
        assert_eq!(p1.instance_dir, p2.instance_dir);
        assert_eq!(p1.agents_dir,   p2.agents_dir);
        assert_eq!(p1.config_dir,   p2.config_dir);
        // Different versioned dirs — concurrent writes are safe.
        assert_ne!(p1.data_dir,             p2.data_dir);
        assert_ne!(p1.cef_cache_dir,        p2.cef_cache_dir);
        assert_ne!(p1.instance_runtime_dir, p2.instance_runtime_dir);
        // Paths contain the version string.
        assert!(p1.data_dir.to_string_lossy().contains("0.40.2"));
        assert!(p2.data_dir.to_string_lossy().contains("0.41.0"));
        let _ = root; // suppress unused warning
    });
}

#[test]
fn the_wan_identity_dir_is_channel_wide_not_per_version() {
    // An upgrade must keep the same WAN instance and agent keys
    // (SPEC_WAN_JEKT_VERIFICATION_2026_09_24.md §2.2).
    with_home_override(|root| {
        clear_channel_env();
        let p1 = DataPaths::resolve("0.40.2", &RuntimeMode::Installed).unwrap();
        let p2 = DataPaths::resolve("0.41.0", &RuntimeMode::Installed).unwrap();
        assert_eq!(p1.wan_identity_dir(), root.join("channels").join("stable").join("wan-identity"));
        assert_eq!(p1.wan_identity_dir(), p2.wan_identity_dir());
        assert!(!p1.wan_identity_dir().starts_with(&p1.data_dir));
    });
}

#[test]
fn home_dir_resolves_to_root() {
    // The agentmux root (~/.agentmux/ or AGENTMUX_HOME_OVERRIDE)
    // is exposed via DataPaths.home_dir for legacy account-wide
    // state like the launcher's config.toml. Resolve in both
    // installed and dev modes; both should point at the same root.
    with_home_override(|root| {
        clear_channel_env();
        let inst = DataPaths::resolve("0.33.641", &RuntimeMode::Installed).unwrap();
        assert_eq!(inst.home_dir, root);
        let dev = DataPaths::resolve(
            "0.33.641",
            &RuntimeMode::Dev {
                branch: "main".into(),
                clone_id: None,
            },
        )
        .unwrap();
        assert_eq!(dev.home_dir, root);
    });
}

#[test]
fn portable_paths_match_installed() {
    with_home_override(|root| {
        clear_channel_env();
        let inst = DataPaths::resolve("0.33.639", &RuntimeMode::Installed).unwrap();
        let port = DataPaths::resolve("0.33.639", &RuntimeMode::Portable).unwrap();
        // Portable + Installed share a default channel (no env
        // override → both fall through to BUILD_CHANNEL_DEFAULT),
        // so their data dirs are the same. Multi-instance
        // isolation is now channel-keyed: if you want two
        // independent portables, override AGENTMUX_CHANNEL on one.
        assert_eq!(inst.channel, port.channel);
        assert_eq!(inst.instance_dir, port.instance_dir);
        assert_eq!(inst.data_dir, port.data_dir);
        // shared/ is mode-independent.
        assert_eq!(inst.shared_dir, root.join("shared"));
        assert_eq!(port.shared_dir, root.join("shared"));
    });
}

#[test]
fn dev_paths_under_branch_and_clone_id() {
    // Two clones of the same branch must resolve to distinct
    // instance dirs when clone_id is supplied. Same branch +
    // different clone_id → different paths → distinct lockfile
    // and pipe namespaces downstream.
    with_home_override(|root| {
        clear_channel_env();
        let a = DataPaths::resolve(
            "0.39.0",
            &RuntimeMode::Dev {
                branch: "main".into(),
                clone_id: Some("aaaaaaaa00000000".into()),
            },
        )
        .unwrap();
        let b = DataPaths::resolve(
            "0.39.0",
            &RuntimeMode::Dev {
                branch: "main".into(),
                clone_id: Some("bbbbbbbb00000000".into()),
            },
        )
        .unwrap();
        assert_eq!(
            a.instance_dir,
            root.join("dev").join("main").join("aaaaaaaa00000000")
        );
        assert_eq!(
            b.instance_dir,
            root.join("dev").join("main").join("bbbbbbbb00000000")
        );
        assert_ne!(a.instance_dir, b.instance_dir);
        assert_eq!(a.channel, "dev-main-aaaaaaaa00000000");
        assert_eq!(b.channel, "dev-main-bbbbbbbb00000000");
    });
}

#[test]
fn dev_paths_legacy_two_level_when_clone_id_none() {
    // Backward compat: a Dev variant without clone_id (e.g.
    // constructed by an older launcher binary, or by the
    // env-string parser) MUST land at the pre-PR two-level dev
    // path so existing in-flight dev sessions don't lose their
    // state on first launch after the upgrade.
    with_home_override(|root| {
        clear_channel_env();
        let p = DataPaths::resolve(
            "0.39.0",
            &RuntimeMode::Dev {
                branch: "main".into(),
                clone_id: None,
            },
        )
        .unwrap();
        assert_eq!(p.instance_dir, root.join("dev").join("main"));
        assert_eq!(p.channel, "dev-main");
    });
}

#[test]
fn dev_paths_under_dev_branch() {
    with_home_override(|root| {
        clear_channel_env();
        let mode = RuntimeMode::Dev {
            branch: "main".into(),
            clone_id: None,
        };
        let p = DataPaths::resolve("0.33.639", &mode).unwrap();
        // Dev mode default: on-disk path stays under dev/<branch>/
        // (unchanged from Phase 1), channel name is "dev-<branch>"
        // for diagnostics. The two diverge intentionally — see
        // resolve_channel_and_dir doc.
        assert_eq!(p.channel, "dev-main");
        assert_eq!(p.instance_dir, root.join("dev").join("main"));
        assert_eq!(p.data_dir, root.join("dev").join("main").join("data"));
        assert_eq!(p.shared_dir, root.join("shared"));
    });
}

#[test]
fn env_override_redirects_any_mode_under_channels() {
    // AGENTMUX_CHANNEL is absolute precedence. Even Dev mode,
    // which would otherwise land at dev/<branch>/, lands under
    // channels/<override>/ when the env is set. This is the
    // "test a hot-fix build against the live stable data" path
    // from SPEC_DATA_CHANNELS_2026_05_24.md §2.2.
    with_home_override(|root| {
        std::env::set_var("AGENTMUX_CHANNEL", "experiment");
        // Cleanup via Drop so a test panic doesn't leak it.
        struct ChannelGuard;
        impl Drop for ChannelGuard {
            fn drop(&mut self) {
                std::env::remove_var("AGENTMUX_CHANNEL");
            }
        }
        let _g = ChannelGuard;

        let inst = DataPaths::resolve("0.33.639", &RuntimeMode::Installed).unwrap();
        assert_eq!(inst.channel, "experiment");
        assert_eq!(inst.instance_dir, root.join("channels").join("experiment"));

        let port = DataPaths::resolve("0.33.639", &RuntimeMode::Portable).unwrap();
        assert_eq!(port.channel, "experiment");
        assert_eq!(port.instance_dir, root.join("channels").join("experiment"));

        let dev = DataPaths::resolve(
            "0.33.639",
            &RuntimeMode::Dev { branch: "main".into(), clone_id: None },
        )
        .unwrap();
        // Override beats the dev/<branch>/ default — channel name
        // matches the override, path lands under channels/, not dev/.
        assert_eq!(dev.channel, "experiment");
        assert_eq!(dev.instance_dir, root.join("channels").join("experiment"));
    });
}

#[test]
fn env_override_rejects_unsafe_or_reserved_names() {
    with_home_override(|_root| {
        // Reserved (would collide with sibling dirs / inner dirs).
        for bad in ["shared", "snapshots", "dev", "versions", "channels", "runtime"] {
            std::env::set_var("AGENTMUX_CHANNEL", bad);
            let r = DataPaths::resolve("0.33.639", &RuntimeMode::Installed);
            std::env::remove_var("AGENTMUX_CHANNEL");
            assert!(
                r.is_err(),
                "AGENTMUX_CHANNEL={:?} should be rejected as reserved",
                bad
            );
        }

        // Path-unsafe (traversal, separators, Windows-reserved).
        // NUL not tested here — Windows' WinAPI rejects NUL in
        // env-var values at the syscall level, so `set_var` would
        // panic before our sanitizer runs. NUL rejection is
        // covered by the direct-call sanitize_path_segment path
        // in identity_dir_rejects_unsafe_segments.
        for bad in ["..", ".", "a/b", "a\\b", "C:foo", "a*b"] {
            std::env::set_var("AGENTMUX_CHANNEL", bad);
            let r = DataPaths::resolve("0.33.639", &RuntimeMode::Installed);
            std::env::remove_var("AGENTMUX_CHANNEL");
            assert!(
                r.is_err(),
                "AGENTMUX_CHANNEL={:?} should be rejected as path-unsafe",
                bad
            );
        }

        // Empty string treated as "not set" — falls through to
        // mode-based default. Documents the behavior so a
        // `.env`-set empty value doesn't surprise.
        std::env::set_var("AGENTMUX_CHANNEL", "");
        let r = DataPaths::resolve("0.33.639", &RuntimeMode::Installed);
        std::env::remove_var("AGENTMUX_CHANNEL");
        assert!(r.is_ok(), "empty AGENTMUX_CHANNEL should fall through to default");
        assert_eq!(r.unwrap().channel, "stable");
    });
}

#[test]
fn env_override_default_is_synonym_for_stable() {
    with_home_override(|root| {
        std::env::set_var("AGENTMUX_CHANNEL", "default");
        let r = DataPaths::resolve("0.33.639", &RuntimeMode::Installed);
        std::env::remove_var("AGENTMUX_CHANNEL");
        let r = r.unwrap();
        // "default" maps to "stable" per spec §7.5; on-disk path
        // is channels/stable/, not channels/default/.
        assert_eq!(r.channel, "stable");
        assert_eq!(r.instance_dir, root.join("channels").join("stable"));
    });
}

#[test]
fn channel_name_length_capped_at_64() {
    with_home_override(|_root| {
        // 64 chars OK, 65 rejected. The cap is for UI sanity, not
        // security — channel names show up in logs and the
        // launcher splash.
        let ok = "a".repeat(64);
        std::env::set_var("AGENTMUX_CHANNEL", &ok);
        let r = DataPaths::resolve("0.33.639", &RuntimeMode::Installed);
        std::env::remove_var("AGENTMUX_CHANNEL");
        assert!(r.is_ok(), "64-char channel should be accepted");

        let too_long = "a".repeat(65);
        std::env::set_var("AGENTMUX_CHANNEL", &too_long);
        let r = DataPaths::resolve("0.33.639", &RuntimeMode::Installed);
        std::env::remove_var("AGENTMUX_CHANNEL");
        assert!(r.is_err(), "65-char channel should be rejected");
    });
}

#[test]
fn dev_branch_traversal_via_runtime_mode_still_rejected() {
    // Dev mode resolution sanitizes the branch via the same
    // sanitize_path_segment as before — channel rename doesn't
    // weaken the traversal-safety guarantees. Reproduces the
    // pre-channel test for parity.
    with_home_override(|_root| {
        clear_channel_env();
        let r = DataPaths::resolve(
            "0.33.639",
            &RuntimeMode::Dev { branch: "..".into(), clone_id: None },
        );
        assert!(r.is_err());
        let r = DataPaths::resolve(
            "0.33.639",
            &RuntimeMode::Dev { branch: "foo/bar".into(), clone_id: None },
        );
        assert!(r.is_err());
    });
}

#[test]
fn identity_dir_rejects_unsafe_segments() {
    // bundle_id flows from auth.start request bodies into
    // create_dir_all. Without sanitization a crafted id would
    // escape the identities root. The function must return None
    // for traversal attempts, separator-bearing segments, and
    // Windows-reserved characters. codex P1 follow-up on #981.
    with_home_override(|_root| {
        let p = DataPaths::resolve("0.33.639", &RuntimeMode::Installed).unwrap();

        // Happy path — a normal UUID-shaped id resolves.
        assert!(p.identity_dir("abc-123-uuid").is_some());

        // Path traversal.
        assert_eq!(p.identity_dir(".."), None);
        assert_eq!(p.identity_dir("."), None);
        assert_eq!(p.identity_dir("../../../etc"), None);
        assert_eq!(p.identity_dir("a/b"), None);
        assert_eq!(p.identity_dir("a\\b"), None);

        // Empty / whitespace-only.
        assert_eq!(p.identity_dir(""), None);
        assert_eq!(p.identity_dir("   "), None);

        // Windows-reserved characters.
        assert_eq!(p.identity_dir("C:foo"), None);
        assert_eq!(p.identity_dir("foo*bar"), None);
        assert_eq!(p.identity_dir("foo?bar"), None);
        assert_eq!(p.identity_dir("with\0nul"), None);
    });
}

#[test]
fn provider_auth_dir_is_shared_and_channel_independent() {
    // The DEFAULT provider auth lives under shared_dir, so it resolves to the
    // SAME path regardless of channel / version / mode — the structural fix
    // for the per-channel "validate-spin" regression. It must NOT live under
    // the per-channel config dir (which is where the regression put it).
    // Retro: docs/retro/retro-provider-auth-isolation-regression-2026-06-05.md
    with_home_override(|_root| {
        clear_channel_env();
        let installed = DataPaths::resolve("0.42.0", &RuntimeMode::Installed).unwrap();
        let dev = DataPaths::resolve(
            "0.42.0",
            &RuntimeMode::Dev { branch: "some-branch".into(), clone_id: None },
        )
        .unwrap();

        let a = installed.provider_auth_dir("claude");
        assert_eq!(
            a,
            dev.provider_auth_dir("claude"),
            "provider auth dir must not vary by channel / mode (instance-independent)"
        );
        assert!(
            a.ends_with("shared/providers/claude"),
            "provider auth dir must live under shared/providers/: {a:?}"
        );
        assert!(
            !a.starts_with(&installed.config_dir),
            "provider auth dir must NOT be under the per-channel config dir"
        );
    });
}

/// RAII guard clearing `AGENTMUX_ISOLATED_AUTH` on drop, even on panic —
/// mirrors `HomeOverrideGuard` above.
struct IsolatedAuthGuard;
impl Drop for IsolatedAuthGuard {
    fn drop(&mut self) {
        std::env::remove_var("AGENTMUX_ISOLATED_AUTH");
    }
}

#[test]
fn identities_dir_is_shared_on_stable_channel() {
    // Auth is shared on every channel by default
    // (SPEC_SHARED_AUTH_ACROSS_CHANNELS_2026_10_03.md); this pins the real
    // release channel. AGENTMUX_CHANNEL is set to "stable" explicitly
    // (mirroring what a real host/srv process always has via from_env(),
    // per to_env_vars()) rather than left unset, so the test names the
    // "stable" channel, not the "channel unknown"
    // fallback covered by identities_dir_is_shared_when_channel_unset.
    with_home_override(|_root| {
        clear_channel_env();
        std::env::remove_var("AGENTMUX_ISOLATED_AUTH");
        let installed = DataPaths::resolve("0.42.0", &RuntimeMode::Installed).unwrap();
        assert_eq!(installed.channel, "stable");
        std::env::set_var("AGENTMUX_CHANNEL", "stable");

        assert!(installed.identities_dir().ends_with("shared/identities"));
        std::env::remove_var("AGENTMUX_CHANNEL");
    });
}

#[test]
fn identities_dir_is_shared_by_default_on_a_non_stable_channel() {
    // SPEC_SHARED_AUTH_ACROSS_CHANNELS_2026_10_03.md: a task-dev branch, a
    // task-package local build and a custom AGENTMUX_CHANNEL all share the
    // global identities with NO explicit AGENTMUX_ISOLATED_AUTH set. This
    // was the opposite until 2026-10-03.
    with_home_override(|_root| {
        clear_channel_env();
        std::env::remove_var("AGENTMUX_ISOLATED_AUTH");
        let dev = DataPaths::resolve(
            "0.42.0",
            &RuntimeMode::Dev { branch: "some-branch".into(), clone_id: None },
        )
        .unwrap();
        std::env::set_var("AGENTMUX_CHANNEL", &dev.channel);

        assert!(dev.identities_dir().ends_with("shared/identities"));
        assert!(
            dev.identities_dir().starts_with(&dev.shared_dir),
            "a shared-by-default identities_dir lives under the global shared_dir"
        );
        std::env::remove_var("AGENTMUX_CHANNEL");
    });
}

#[test]
fn identities_dir_is_shared_when_channel_unset() {
    // Conservative fallback: no AGENTMUX_CHANNEL in the process env
    // at all (e.g. a bare `cargo test`/`cargo run` outside the
    // launcher's from_env() chain) — stay global rather than guess.
    with_home_override(|_root| {
        clear_channel_env();
        std::env::remove_var("AGENTMUX_ISOLATED_AUTH");
        let dev = DataPaths::resolve(
            "0.42.0",
            &RuntimeMode::Dev { branch: "some-branch".into(), clone_id: None },
        )
        .unwrap();
        // AGENTMUX_CHANNEL deliberately left unset here, unlike the
        // non-stable-channel test above.

        assert!(dev.identities_dir().ends_with("shared/identities"));
    });
}

#[test]
fn identities_dir_is_per_channel_when_isolated_auth_explicitly_set() {
    with_home_override(|_root| {
        clear_channel_env();
        std::env::set_var("AGENTMUX_ISOLATED_AUTH", "1");
        let _guard = IsolatedAuthGuard;

        let dev_a = DataPaths::resolve(
            "0.42.0",
            &RuntimeMode::Dev { branch: "branch-a".into(), clone_id: None },
        )
        .unwrap();
        let dev_b = DataPaths::resolve(
            "0.42.0",
            &RuntimeMode::Dev { branch: "branch-b".into(), clone_id: None },
        )
        .unwrap();

        assert_ne!(
            dev_a.identities_dir(),
            dev_b.identities_dir(),
            "isolated identities_dir must differ per channel"
        );
        assert_eq!(dev_a.identities_dir(), dev_a.instance_dir.join("identities"));
        assert!(
            !dev_a.identities_dir().starts_with(&dev_a.shared_dir),
            "isolated identities_dir must NOT live under the global shared_dir"
        );
    });
}

#[test]
fn identities_dir_is_shared_when_isolated_auth_explicitly_disabled_on_non_stable_channel() {
    // The escape hatch: AGENTMUX_ISOLATED_AUTH=0 overrides the new
    // channel-based default back to global sharing, even on a
    // non-stable channel.
    with_home_override(|_root| {
        clear_channel_env();
        std::env::set_var("AGENTMUX_ISOLATED_AUTH", "0");
        let _guard = IsolatedAuthGuard;

        let dev = DataPaths::resolve(
            "0.42.0",
            &RuntimeMode::Dev { branch: "some-branch".into(), clone_id: None },
        )
        .unwrap();
        std::env::set_var("AGENTMUX_CHANNEL", &dev.channel);

        assert!(dev.identities_dir().ends_with("shared/identities"));
        std::env::remove_var("AGENTMUX_CHANNEL");
    });
}

#[test]
fn isolated_auth_reason_classifies_all_three_states() {
    let _lock = TEST_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    std::env::remove_var("AGENTMUX_ISOLATED_AUTH");
    clear_channel_env();

    std::env::set_var("AGENTMUX_ISOLATED_AUTH", "1");
    assert_eq!(isolated_auth_reason(), IsolatedAuthReason::ExplicitOptIn);
    assert!(isolated_auth_reason().is_isolated());

    std::env::set_var("AGENTMUX_ISOLATED_AUTH", "0");
    assert_eq!(isolated_auth_reason(), IsolatedAuthReason::ExplicitOptOut);
    assert!(!isolated_auth_reason().is_isolated());

    // No override: shared on EVERY channel, not only `stable`.
    std::env::remove_var("AGENTMUX_ISOLATED_AUTH");
    for channel in ["dev-some-branch", "local-main-b28b7a-051fbf53", "stable", "custom-channel"] {
        std::env::set_var("AGENTMUX_CHANNEL", channel);
        assert_eq!(isolated_auth_reason(), IsolatedAuthReason::DefaultShared, "{channel}");
        assert!(!isolated_auth_reason().is_isolated(), "{channel}");
    }

    clear_channel_env();
    assert_eq!(isolated_auth_reason(), IsolatedAuthReason::DefaultShared);
    assert!(!isolated_auth_reason().is_isolated());
}

#[test]
fn isolated_auth_reason_fails_safe_on_a_malformed_value_on_a_non_stable_channel() {
    // Anything other than exactly "1" is an explicit opt-out (global),
    // matching the original `.map(|v| v == "1")` rule for every non-"1"
    // value (#2431): a misspelled value must never isolate a channel.
    let _lock = TEST_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    clear_channel_env();
    std::env::set_var("AGENTMUX_CHANNEL", "dev-some-branch");

    for malformed in ["false", "no", "TRUE", "2", ""] {
        std::env::set_var("AGENTMUX_ISOLATED_AUTH", malformed);
        assert_eq!(
            isolated_auth_reason(),
            IsolatedAuthReason::ExplicitOptOut,
            "AGENTMUX_ISOLATED_AUTH={malformed:?} on a non-stable channel must fail safe to global, not isolate"
        );
        assert!(!isolated_auth_reason().is_isolated());
    }

    std::env::remove_var("AGENTMUX_ISOLATED_AUTH");
    clear_channel_env();
}

/// RAII guard clearing `AGENTMUX_ISOLATED_SETTINGS` on drop, even on
/// panic — mirrors `IsolatedAuthGuard` above.
struct IsolatedSettingsGuard;
impl Drop for IsolatedSettingsGuard {
    fn drop(&mut self) {
        std::env::remove_var("AGENTMUX_ISOLATED_SETTINGS");
    }
}

#[test]
fn isolated_settings_reason_classifies_all_four_states() {
    let _lock = TEST_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    std::env::remove_var("AGENTMUX_ISOLATED_SETTINGS");
    clear_channel_env();
    let _guard = IsolatedSettingsGuard;

    std::env::set_var("AGENTMUX_ISOLATED_SETTINGS", "1");
    assert_eq!(isolated_settings_reason(), IsolatedSettingsReason::ExplicitOptIn);
    assert!(isolated_settings_reason().is_isolated());

    std::env::set_var("AGENTMUX_ISOLATED_SETTINGS", "0");
    assert_eq!(isolated_settings_reason(), IsolatedSettingsReason::ExplicitOptOut);
    assert!(!isolated_settings_reason().is_isolated());

    std::env::remove_var("AGENTMUX_ISOLATED_SETTINGS");
    std::env::set_var("AGENTMUX_CHANNEL", "local-some-branch-abc123-1");
    assert_eq!(isolated_settings_reason(), IsolatedSettingsReason::ChannelDefaultIsolated);
    assert!(isolated_settings_reason().is_isolated());

    std::env::set_var("AGENTMUX_CHANNEL", "stable");
    assert_eq!(isolated_settings_reason(), IsolatedSettingsReason::ChannelDefaultGlobal);
    assert!(!isolated_settings_reason().is_isolated());

    clear_channel_env();
    assert_eq!(isolated_settings_reason(), IsolatedSettingsReason::ChannelDefaultGlobal);
    assert!(!isolated_settings_reason().is_isolated());
}

#[test]
fn isolated_settings_reason_fails_safe_on_a_malformed_value_on_a_non_stable_channel() {
    // Same fail-safe rule as isolated_auth_reason's equivalent test
    // (reagentx P2 on PR #2431) — a typo'd opt-out must land on
    // ExplicitOptOut (global), not silently isolate.
    let _lock = TEST_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    clear_channel_env();
    std::env::set_var("AGENTMUX_CHANNEL", "dev-some-branch");
    let _guard = IsolatedSettingsGuard;

    for malformed in ["false", "no", "TRUE", "2", ""] {
        std::env::set_var("AGENTMUX_ISOLATED_SETTINGS", malformed);
        assert_eq!(
            isolated_settings_reason(),
            IsolatedSettingsReason::ExplicitOptOut,
            "AGENTMUX_ISOLATED_SETTINGS={malformed:?} on a non-stable channel must fail safe to global, not isolate"
        );
        assert!(!isolated_settings_reason().is_isolated());
    }

    std::env::remove_var("AGENTMUX_ISOLATED_SETTINGS");
    clear_channel_env();
}

#[test]
fn isolated_settings_and_isolated_auth_are_independent_flags() {
    // Open question §1 in the spec: these must not accidentally share
    // state — setting one must not affect the other's resolution.
    let _lock = TEST_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    clear_channel_env();
    std::env::remove_var("AGENTMUX_ISOLATED_AUTH");
    std::env::remove_var("AGENTMUX_ISOLATED_SETTINGS");
    std::env::set_var("AGENTMUX_CHANNEL", "dev-some-branch");
    let _guard_a = IsolatedAuthGuard;
    let _guard_s = IsolatedSettingsGuard;

    std::env::set_var("AGENTMUX_ISOLATED_AUTH", "0");
    assert!(!isolated_auth_enabled());
    assert!(
        isolated_settings_enabled(),
        "AGENTMUX_ISOLATED_AUTH=0 must not disable settings isolation"
    );

    // Auth is shared by default now, so isolate it explicitly to see that
    // turning settings isolation off leaves it alone.
    std::env::set_var("AGENTMUX_ISOLATED_AUTH", "1");
    std::env::set_var("AGENTMUX_ISOLATED_SETTINGS", "0");
    assert!(!isolated_settings_enabled());
    assert!(
        isolated_auth_enabled(),
        "AGENTMUX_ISOLATED_SETTINGS=0 must not disable auth isolation"
    );

    std::env::remove_var("AGENTMUX_ISOLATED_SETTINGS");
    clear_channel_env();
}

/// RAII guard clearing `AGENTMUX_ISOLATED_MUXBUS` on drop, even on
/// panic — mirrors `IsolatedSettingsGuard` above.
struct IsolatedMuxbusReconnectGuard;
impl Drop for IsolatedMuxbusReconnectGuard {
    fn drop(&mut self) {
        std::env::remove_var("AGENTMUX_ISOLATED_MUXBUS");
    }
}

#[test]
fn isolated_muxbus_reconnect_reason_classifies_every_state() {
    let _lock = TEST_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    std::env::remove_var("AGENTMUX_ISOLATED_MUXBUS");
    clear_channel_env();
    let _guard = IsolatedMuxbusReconnectGuard;

    std::env::set_var("AGENTMUX_ISOLATED_MUXBUS", "1");
    assert_eq!(
        isolated_muxbus_reconnect_reason(),
        IsolatedMuxbusReconnectReason::ExplicitOptIn
    );
    assert!(isolated_muxbus_reconnect_reason().is_isolated());

    std::env::set_var("AGENTMUX_ISOLATED_MUXBUS", "0");
    assert_eq!(
        isolated_muxbus_reconnect_reason(),
        IsolatedMuxbusReconnectReason::ExplicitOptOut
    );
    assert!(!isolated_muxbus_reconnect_reason().is_isolated());

    std::env::remove_var("AGENTMUX_ISOLATED_MUXBUS");
    std::env::set_var("AGENTMUX_CHANNEL", "local-some-branch-abc123-1");
    assert_eq!(
        isolated_muxbus_reconnect_reason().is_isolated(),
        keychain_prompts_per_build(),
        "a local channel skips the reconnect only where the keychain prompts"
    );

    std::env::set_var("AGENTMUX_CHANNEL", "stable");
    assert_eq!(
        isolated_muxbus_reconnect_reason(),
        IsolatedMuxbusReconnectReason::ChannelDefaultGlobal
    );
    assert!(!isolated_muxbus_reconnect_reason().is_isolated());

    clear_channel_env();
    assert_eq!(
        isolated_muxbus_reconnect_reason(),
        IsolatedMuxbusReconnectReason::ChannelDefaultGlobal
    );
    assert!(!isolated_muxbus_reconnect_reason().is_isolated());
}

#[test]
fn isolated_muxbus_reconnect_reason_exempts_task_dev_channels() {
    // reagentx P1 on PR #3248 (both rounds): a `dev-<branch>` channel
    // (RuntimeMode::Dev's channel format) must NOT be isolated by
    // default — it's stable/per-branch, not a randomized per-build
    // local-package channel, so it never has the "never-before-seen
    // app to Keychain every time" problem this flag targets, and
    // isolating it anyway silently drops automatic MuxBus reconnect
    // (and WAN notifications) for the entire `task dev` session.
    let _lock = TEST_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    std::env::remove_var("AGENTMUX_ISOLATED_MUXBUS");
    clear_channel_env();
    let _guard = IsolatedMuxbusReconnectGuard;

    for dev_channel in ["dev-main", "dev-some-branch", "dev-some-branch-a1b2c3d4"] {
        std::env::set_var("AGENTMUX_CHANNEL", dev_channel);
        assert_eq!(
            isolated_muxbus_reconnect_reason(),
            IsolatedMuxbusReconnectReason::ChannelDefaultGlobal,
            "dev channel {dev_channel:?} must NOT be isolated by default"
        );
        assert!(!isolated_muxbus_reconnect_reason().is_isolated());
    }

    // A local-package channel (no "dev-" prefix) is still isolated where
    // the keychain prompts — the exemption is specific to "dev-", not
    // "anything but stable".
    assert_eq!(
        crate::muxbus_reconnect::muxbus_reconnect_reason_for(None, Some("local-main-abc123-9"), true),
        IsolatedMuxbusReconnectReason::ChannelDefaultIsolated
    );

    clear_channel_env();
}

#[test]
fn isolated_muxbus_reconnect_reason_fails_safe_on_a_malformed_value_on_a_non_stable_channel() {
    // Same fail-safe rule as isolated_auth_reason's/isolated_settings_reason's
    // equivalent tests (reagentx P2 on PR #2431) — a typo'd opt-out must
    // land on ExplicitOptOut (reconnect stays on), not silently isolate.
    let _lock = TEST_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    clear_channel_env();
    std::env::set_var("AGENTMUX_CHANNEL", "dev-some-branch");
    let _guard = IsolatedMuxbusReconnectGuard;

    for malformed in ["false", "no", "TRUE", "2", ""] {
        std::env::set_var("AGENTMUX_ISOLATED_MUXBUS", malformed);
        assert_eq!(
            isolated_muxbus_reconnect_reason(),
            IsolatedMuxbusReconnectReason::ExplicitOptOut,
            "AGENTMUX_ISOLATED_MUXBUS={malformed:?} on a non-stable channel must fail safe to global, not isolate"
        );
        assert!(!isolated_muxbus_reconnect_reason().is_isolated());
    }

    std::env::remove_var("AGENTMUX_ISOLATED_MUXBUS");
    clear_channel_env();
}

#[test]
fn isolated_muxbus_reconnect_is_independent_of_auth_and_settings_isolation() {
    // Same independence guarantee as
    // isolated_settings_and_isolated_auth_are_independent_flags — a
    // separate flag on purpose (see the production doc comment).
    let _lock = TEST_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    clear_channel_env();
    std::env::remove_var("AGENTMUX_ISOLATED_AUTH");
    std::env::remove_var("AGENTMUX_ISOLATED_SETTINGS");
    std::env::remove_var("AGENTMUX_ISOLATED_MUXBUS");
    // NOT "dev-some-branch": muxbus reconnect deliberately exempts
    // dev-* channels (see isolated_muxbus_reconnect_reason's doc
    // comment) while auth/settings do not — a dev channel would make
    // this independence test's own channel-default assertions below
    // wrong for muxbus specifically, unrelated to what this test
    // actually checks (that the three env-var overrides don't leak
    // into each other). A local-package-shaped channel isolates settings
    // and the muxbus reconnect by their channel default; auth is opted in
    // explicitly below, since it is shared by default.
    std::env::set_var("AGENTMUX_CHANNEL", "local-some-branch-abc123-1");
    let _guard_a = IsolatedAuthGuard;
    let _guard_s = IsolatedSettingsGuard;
    let _guard_m = IsolatedMuxbusReconnectGuard;

    std::env::set_var("AGENTMUX_ISOLATED_AUTH", "1");
    std::env::set_var("AGENTMUX_ISOLATED_MUXBUS", "0");
    assert!(!isolated_muxbus_reconnect_enabled());
    assert!(
        isolated_auth_enabled(),
        "AGENTMUX_ISOLATED_MUXBUS=0 must not disable auth isolation"
    );
    assert!(
        isolated_settings_enabled(),
        "AGENTMUX_ISOLATED_MUXBUS=0 must not disable settings isolation"
    );

    // Opted in explicitly: a local channel's own default depends on the OS.
    std::env::set_var("AGENTMUX_ISOLATED_MUXBUS", "1");
    std::env::set_var("AGENTMUX_ISOLATED_AUTH", "0");
    assert!(!isolated_auth_enabled());
    assert!(
        isolated_muxbus_reconnect_enabled(),
        "AGENTMUX_ISOLATED_AUTH=0 must not disable muxbus-reconnect isolation"
    );
    std::env::remove_var("AGENTMUX_ISOLATED_MUXBUS");

    std::env::remove_var("AGENTMUX_ISOLATED_AUTH");
    clear_channel_env();
}

#[test]
fn ensure_dirs_creates_everything() {
    with_home_override(|_root| {
        clear_channel_env();
        let p = DataPaths::resolve("0.33.639", &RuntimeMode::Installed).unwrap();
        p.ensure_dirs().unwrap();
        assert!(p.instance_dir.is_dir());
        assert!(p.data_dir.is_dir());
        assert!(p.data_dir.join("db").is_dir());
        assert!(p.config_dir.is_dir());
        assert!(p.logs_dir.is_dir());
        assert!(p.cef_cache_dir.is_dir());
        assert!(p.agents_dir.is_dir());
        assert!(p.instance_runtime_dir.is_dir());
        assert!(p.shared_dir.is_dir());
    });
}

#[test]
fn env_vars_round_trip() {
    with_home_override(|_root| {
        clear_channel_env();
        let p1 = DataPaths::resolve(
            "0.33.639",
            &RuntimeMode::Dev {
                branch: "main".into(),
                clone_id: None,
            },
        )
        .unwrap();
        // Apply each env var, then read back.
        for (k, v) in p1.to_env_vars() {
            std::env::set_var(k, v);
        }
        let p2 = DataPaths::from_env().expect("round-trip");
        assert_eq!(p1.instance_dir, p2.instance_dir);
        assert_eq!(p1.data_dir, p2.data_dir);
        assert_eq!(p1.shared_dir, p2.shared_dir);
        assert_eq!(p1.mode, p2.mode);
        assert_eq!(p1.channel, p2.channel);
        // Cleanup
        for (k, _) in p1.to_env_vars() {
            std::env::remove_var(k);
        }
    });
}

#[test]
fn resolve_rejects_dev_branch_traversal() {
    // Even if a caller manages to construct a Dev variant with an
    // unsafe branch (bypassing parse_mode_string sanitization),
    // resolve() must catch it.
    with_home_override(|_root| {
        clear_channel_env();
        let mode = RuntimeMode::Dev {
            branch: "..".into(),
            clone_id: None,
        };
        assert!(DataPaths::resolve("0.33.639", &mode).is_err());
        let mode = RuntimeMode::Dev {
            branch: "foo/bar".into(),
            clone_id: None,
        };
        assert!(DataPaths::resolve("0.33.639", &mode).is_err());
    });
}

#[test]
fn resolve_rejects_traversal_version() {
    with_home_override(|_root| {
        clear_channel_env();
        assert!(DataPaths::resolve("..", &RuntimeMode::Installed).is_err());
        assert!(DataPaths::resolve(
            "0.33.639/etc",
            &RuntimeMode::Installed
        )
        .is_err());
        // Drive-relative on Windows: `PathBuf::join("versions")
        // .join("C:temp")` would resolve outside the intended
        // ~/.agentmux/versions/ subtree because `C:temp` is a
        // drive-relative path, not a literal filename.
        assert!(DataPaths::resolve("C:temp", &RuntimeMode::Installed).is_err());
        // Other Windows-reserved chars also rejected.
        for v in ["a*b", "a?b", "a|b", "a<b", "a>b", "a\"b"] {
            assert!(
                DataPaths::resolve(v, &RuntimeMode::Installed).is_err(),
                "should reject version with reserved char: {:?}",
                v
            );
        }
    });
}

#[test]
fn from_env_fails_fast_on_missing_vars() {
    let _g = TEST_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    // Clear all expected vars.
    for k in [
        "AGENTMUX_INSTANCE_DIR",
        "AGENTMUX_DATA_DIR",
        "AGENTMUX_CONFIG_DIR",
        "AGENTMUX_LOG_DIR",
        "AGENTMUX_CEF_CACHE_DIR",
        "AGENTMUX_AGENTS_DIR",
        "AGENTMUX_INSTANCE_RUNTIME_DIR",
        "AGENTMUX_SHARED_DIR",
        "AGENTMUX_RUNTIME_MODE",
        "AGENTMUX_CHANNEL",
    ] {
        std::env::remove_var(k);
    }
    assert!(DataPaths::from_env().is_none());
}

#[test]
fn from_env_fails_fast_when_channel_missing() {
    // Symmetric to from_env_fails_fast_on_missing_vars but
    // isolates the channel-specific case: a launcher built with
    // the new code will always export AGENTMUX_CHANNEL; a missing
    // value indicates a launcher/srv version skew that must fail
    // loudly rather than silently fall back to a wrong-channel
    // write. Pre-set all other vars to confirm channel is what's
    // gating.
    let _g = TEST_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    for (k, v) in [
        ("AGENTMUX_INSTANCE_DIR", "/tmp/x"),
        ("AGENTMUX_DATA_DIR", "/tmp/x/data"),
        ("AGENTMUX_CONFIG_DIR", "/tmp/x/config"),
        ("AGENTMUX_LOG_DIR", "/tmp/x/logs"),
        ("AGENTMUX_CEF_CACHE_DIR", "/tmp/x/cef"),
        ("AGENTMUX_AGENTS_DIR", "/tmp/x/agents"),
        ("AGENTMUX_INSTANCE_RUNTIME_DIR", "/tmp/x/runtime"),
        ("AGENTMUX_SHARED_DIR", "/tmp/x/shared"),
        ("AGENTMUX_RUNTIME_MODE", "installed"),
    ] {
        std::env::set_var(k, v);
    }
    std::env::remove_var("AGENTMUX_CHANNEL");
    assert!(
        DataPaths::from_env().is_none(),
        "from_env() must refuse when AGENTMUX_CHANNEL is missing"
    );
    // Cleanup
    for k in [
        "AGENTMUX_INSTANCE_DIR",
        "AGENTMUX_DATA_DIR",
        "AGENTMUX_CONFIG_DIR",
        "AGENTMUX_LOG_DIR",
        "AGENTMUX_CEF_CACHE_DIR",
        "AGENTMUX_AGENTS_DIR",
        "AGENTMUX_INSTANCE_RUNTIME_DIR",
        "AGENTMUX_SHARED_DIR",
        "AGENTMUX_RUNTIME_MODE",
    ] {
        std::env::remove_var(k);
    }
}

#[test]
fn resolve_path_only_ignores_env_channel() {
    // resolve_path_only is the dev-build / nested-launch self-detection
    // variant — it deliberately ignores AGENTMUX_CHANNEL because a host (dev
    // OR a portable launched nested inside another AgentMux) inherits the
    // parent's channel env and would cross-contaminate.
    //
    // Codex P1 regression test on PR #1027 (dev), extended for the nested
    // portable case the launcher relies on (crates/launcher/src/data_dir.rs).
    with_home_override(|root| {
        // Use a NON-default channel value so "ignored" (→ baked default) is
        // distinguishable from "honored" (→ this value). BUILD_CHANNEL_DEFAULT
        // is "stable" in tests, so an override of "beta" that gets ignored
        // resolves to "stable" — a name the env never set. (A "stable" override
        // would be tautological against the default.)
        std::env::set_var("AGENTMUX_CHANNEL", "beta");
        struct ChannelGuard;
        impl Drop for ChannelGuard {
            fn drop(&mut self) {
                std::env::remove_var("AGENTMUX_CHANNEL");
            }
        }
        let _g = ChannelGuard;

        let dev = DataPaths::resolve_path_only(
            "0.33.639",
            &RuntimeMode::Dev { branch: "main".into(), clone_id: None },
        )
        .unwrap();
        // Dev path_only ignores "beta" → stays under dev/main/.
        assert_eq!(dev.channel, "dev-main");
        assert_eq!(dev.instance_dir, root.join("dev").join("main"));

        let inst = DataPaths::resolve_path_only(
            "0.33.639",
            &RuntimeMode::Installed,
        )
        .unwrap();
        // Installed path_only ignores "beta" → baked default "stable".
        assert_eq!(inst.channel, "stable");
        assert_eq!(inst.instance_dir, root.join("channels").join("stable"));

        // Portable path_only ALSO ignores the env channel — the behavior the
        // launcher relies on for a NESTED portable launch. A build launched
        // inside another AgentMux pane inherits AGENTMUX_CHANNEL=<parent> and
        // must resolve to its OWN baked channel, NOT the leaked one — else it
        // adopts the parent's data dir + cef-cache and CEF's user-data-dir
        // singleton forwards it into the parent. "beta" ignored → baked
        // "stable".
        let port = DataPaths::resolve_path_only(
            "0.33.639",
            &RuntimeMode::Portable,
        )
        .unwrap();
        assert_eq!(
            port.channel, "stable",
            "nested portable must IGNORE the leaked AGENTMUX_CHANNEL=beta"
        );
        assert_eq!(port.instance_dir, root.join("channels").join("stable"));

        // Sanity / B6: regular `resolve` DOES honor the override in every mode
        // (Dev AND Portable) → "beta". This proves (a) the divergence is solely
        // on the path_only variant, (b) the assertions above are not tautological
        // against the default, and (c) an EXPLICIT standalone AGENTMUX_CHANNEL is
        // still honored for portables (parallel-channel testing, PR #1027).
        let dev_env = DataPaths::resolve(
            "0.33.639",
            &RuntimeMode::Dev { branch: "main".into(), clone_id: None },
        )
        .unwrap();
        assert_eq!(dev_env.channel, "beta");
        assert_eq!(dev_env.instance_dir, root.join("channels").join("beta"));

        let port_env = DataPaths::resolve(
            "0.33.639",
            &RuntimeMode::Portable,
        )
        .unwrap();
        assert_eq!(
            port_env.channel, "beta",
            "standalone portable still HONORS an explicit AGENTMUX_CHANNEL"
        );
        assert_eq!(port_env.instance_dir, root.join("channels").join("beta"));
    });
}

#[test]
fn sanitize_channel_name_accepts_normal_names() {
    // The happy path — make sure stable / beta / local-main
    // and friends all sanitize cleanly. Catches regressions in
    // case the reserved list grows by mistake.
    assert_eq!(sanitize_channel_name("stable"), Some("stable".into()));
    assert_eq!(sanitize_channel_name("beta"), Some("beta".into()));
    assert_eq!(
        sanitize_channel_name("local-main"),
        Some("local-main".into())
    );
    assert_eq!(
        sanitize_channel_name("dev-main"),
        Some("dev-main".into())
    );
    assert_eq!(
        sanitize_channel_name("experiment_42"),
        Some("experiment_42".into())
    );
}

// ensure_history_link — real filesystem operations (junction on
// Windows, symlink on Unix), not mocked, per
// docs/specs/SPEC_AGENT_IDENTITY_HISTORY_PERSISTENCE_PROTOCOL_2026_08_16.md
// §4.1. This is credential-adjacent storage code; these tests
// actually create the link and read a file back THROUGH it, rather
// than only asserting the `Result` came back `Ok`.

#[test]
fn ensure_history_link_makes_files_written_at_target_visible_through_link() {
    let tmp = TempDir::new().expect("tempdir");
    let link_path = tmp.path().join("isolated").join("projects");
    let target_dir = tmp.path().join("global").join("projects");

    ensure_history_link(&link_path, &target_dir).expect("link creation must succeed");

    std::fs::write(target_dir.join("session-1.jsonl"), b"hello").unwrap();
    let via_link = std::fs::read(link_path.join("session-1.jsonl"))
        .expect("a file written at the global target must be readable through the isolated link path");
    assert_eq!(via_link, b"hello");
}

#[test]
fn ensure_history_link_is_idempotent() {
    let tmp = TempDir::new().expect("tempdir");
    let link_path = tmp.path().join("isolated").join("projects");
    let target_dir = tmp.path().join("global").join("projects");

    ensure_history_link(&link_path, &target_dir).unwrap();
    std::fs::write(target_dir.join("session-1.jsonl"), b"hello").unwrap();

    // A second call against the already-correct link must not touch
    // (let alone lose) the file that's already there.
    ensure_history_link(&link_path, &target_dir).expect("re-calling on an already-correct link must succeed");
    assert!(link_path.join("session-1.jsonl").exists());
}

#[test]
fn ensure_history_link_migrates_pre_existing_real_directory_contents() {
    let tmp = TempDir::new().expect("tempdir");
    let link_path = tmp.path().join("isolated").join("projects");
    let target_dir = tmp.path().join("global").join("projects");

    // Simulate pre-fix state: link_path is a REAL directory with real
    // session data already in it (written before this function
    // existed / before isolation applied to this bundle).
    std::fs::create_dir_all(&link_path).unwrap();
    std::fs::write(link_path.join("old-session.jsonl"), b"pre-existing history").unwrap();

    ensure_history_link(&link_path, &target_dir).expect("migration + link creation must succeed");

    // The old file must now live at the global target, not be lost,
    // and must still be reachable via the (now-linked) original path.
    assert_eq!(
        std::fs::read(target_dir.join("old-session.jsonl")).unwrap(),
        b"pre-existing history",
        "pre-existing history must be migrated to the global target, not lost"
    );
    assert_eq!(
        std::fs::read(link_path.join("old-session.jsonl")).unwrap(),
        b"pre-existing history",
        "migrated history must still be reachable at the original (now-linked) path"
    );
}

#[test]
fn ensure_history_link_never_overwrites_a_name_collision_at_the_target() {
    let tmp = TempDir::new().expect("tempdir");
    let link_path = tmp.path().join("isolated").join("projects");
    let target_dir = tmp.path().join("global").join("projects");

    std::fs::create_dir_all(&target_dir).unwrap();
    std::fs::write(target_dir.join("session-1.jsonl"), b"global version").unwrap();
    std::fs::create_dir_all(&link_path).unwrap();
    std::fs::write(link_path.join("session-1.jsonl"), b"isolated version -- must not be lost").unwrap();

    // The link can't be created while link_path is a non-empty real
    // directory (the colliding entry blocks the migration's cleanup
    // remove_dir) -- this must surface as an error, not silently
    // clobber either copy.
    let result = ensure_history_link(&link_path, &target_dir);
    assert!(result.is_err(), "a name collision must surface as an error, not silently pick a winner");

    // Neither copy was touched.
    assert_eq!(std::fs::read(target_dir.join("session-1.jsonl")).unwrap(), b"global version");
    assert_eq!(
        std::fs::read(link_path.join("session-1.jsonl")).unwrap(),
        b"isolated version -- must not be lost"
    );
}

// reagentx P2 on PR #2605: the previous version of this function's
// fallback match arm treated "a plain file at link_path" the same as
// "a stale link at link_path" and deleted it outright with no
// migration, contradicting the function's own doc comment.
#[test]
fn ensure_history_link_refuses_to_delete_a_plain_file_at_the_link_path() {
    let tmp = TempDir::new().expect("tempdir");
    let link_path = tmp.path().join("isolated").join("projects");
    let target_dir = tmp.path().join("global").join("projects");

    std::fs::create_dir_all(link_path.parent().unwrap()).unwrap();
    std::fs::write(&link_path, b"unexpected real file, not a directory or a link").unwrap();

    let result = ensure_history_link(&link_path, &target_dir);
    assert!(result.is_err(), "a plain file at the link path must surface as an error, never be silently deleted");
    assert_eq!(
        std::fs::read(&link_path).unwrap(),
        b"unexpected real file, not a directory or a link",
        "the file must still exist, untouched, after the refused operation"
    );
}

#[test]
fn ensure_history_link_repoints_a_link_to_somewhere_else_and_leaves_the_old_target_alone() {
    let tmp = TempDir::new().expect("tempdir");
    let link_path = tmp.path().join("isolated").join("projects");
    let old_target = tmp.path().join("old").join("projects");
    let target_dir = tmp.path().join("global").join("projects");

    ensure_history_link(&link_path, &old_target).unwrap();
    std::fs::write(old_target.join("old.jsonl"), b"old").unwrap();
    std::fs::create_dir_all(&target_dir).unwrap();
    std::fs::write(target_dir.join("new.jsonl"), b"new").unwrap();

    ensure_history_link(&link_path, &target_dir).expect("a stale link is replaced");

    assert_eq!(std::fs::read(link_path.join("new.jsonl")).unwrap(), b"new");
    assert!(!link_path.join("old.jsonl").exists(), "the link now leads to the new target only");
    assert_eq!(std::fs::read(old_target.join("old.jsonl")).unwrap(), b"old", "the old target's data is not touched");
}

// chatgpt-codex-connector + reagentx both claimed on PR #2605 that
// `std::fs::symlink_metadata` reports a Windows junction as
// `is_dir()=true, is_symlink()=false` — which, if true, would mean
// `ensure_history_link`'s `meta.is_dir() && !meta.file_type().is_symlink()`
// migration-branch check incorrectly treats a stale junction as a
// real directory. Verified empirically (twice, independently) that
// this claim is FALSE for the actual Rust std + toolchain this crate
// builds with: a real junction reports `is_dir()=false,
// is_symlink()=true` via `symlink_metadata` — `is_dir()=true` only
// shows up via `fs::metadata` (which follows the link, as
// documented). Kept as a permanent test, not deleted after
// resolving the review thread, so a future "fix" for this
// non-existent bug doesn't silently reintroduce one by "fixing"
// something that was already correct.
#[cfg(windows)]
#[test]
fn windows_junction_is_reported_as_symlink_not_as_a_real_directory() {
    let tmp = TempDir::new().expect("tempdir");
    let link = tmp.path().join("isolated").join("linked");
    let target = tmp.path().join("global");
    ensure_history_link(&link, &target).unwrap();

    assert!(junction::exists(&link).unwrap(), "sanity: this must actually be a junction");

    let meta = std::fs::symlink_metadata(&link).unwrap();
    assert!(
        meta.file_type().is_symlink(),
        "a Windows junction must report is_symlink()=true via symlink_metadata"
    );
    assert!(
        !meta.is_dir(),
        "a Windows junction must NOT report is_dir()=true via symlink_metadata (only via the \
         link-following fs::metadata) — if this ever changes, ensure_history_link's migration \
         branch needs the junction::exists() check the (currently incorrect) review claimed was missing"
    );
}
// ── #3682: the AgentMux root is owner-only on Unix ──────────────────────

#[cfg(unix)]
fn mode_of(p: &Path) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(p).unwrap().permissions().mode() & 0o777
}

#[cfg(unix)]
fn set_mode(p: &Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(p, std::fs::Permissions::from_mode(mode)).unwrap();
}

#[cfg(unix)]
#[test]
fn owner_only_creates_a_missing_root_0700() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().join("parent").join(".agentmux");
    assert_eq!(ensure_owner_only_dir_inner(&root, None).unwrap(), OwnerOnlyOutcome::Created);
    assert_eq!(mode_of(&root), 0o700);
}

#[cfg(unix)]
#[test]
fn owner_only_leaves_parents_it_had_to_create_at_the_normal_umask() {
    // Only the root is AgentMux's to make private; a parent dir it had to
    // create (a custom AGENTMUX_DATA_HOME) belongs to the user.
    let tmp = TempDir::new().unwrap();
    let parent = tmp.path().join("parent");
    let root = parent.join(".agentmux");
    ensure_owner_only_dir_inner(&root, None).unwrap();
    assert_ne!(mode_of(&parent), 0o700, "parent must not be forced owner-only");
}

#[cfg(unix)]
#[test]
fn owner_only_tightens_an_existing_world_readable_root() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().join(".agentmux");
    std::fs::create_dir(&root).unwrap();
    set_mode(&root, 0o755);
    assert_eq!(
        ensure_owner_only_dir_inner(&root, None).unwrap(),
        OwnerOnlyOutcome::Tightened { previous_mode: 0o755 }
    );
    assert_eq!(mode_of(&root), 0o700);
}

#[cfg(unix)]
#[test]
fn owner_only_keeps_the_owner_bits_it_found() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().join(".agentmux");
    std::fs::create_dir(&root).unwrap();
    set_mode(&root, 0o570);
    assert_eq!(
        ensure_owner_only_dir_inner(&root, None).unwrap(),
        OwnerOnlyOutcome::Tightened { previous_mode: 0o570 }
    );
    assert_eq!(mode_of(&root), 0o500);
    set_mode(&root, 0o700); // so TempDir can clean up
}

#[cfg(unix)]
#[test]
fn owner_only_is_a_no_op_on_an_already_private_root() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().join(".agentmux");
    std::fs::create_dir(&root).unwrap();
    set_mode(&root, 0o700);
    assert_eq!(ensure_owner_only_dir_inner(&root, None).unwrap(), OwnerOnlyOutcome::AlreadyPrivate);
    assert_eq!(mode_of(&root), 0o700);
}

#[cfg(unix)]
#[test]
fn owner_only_never_touches_the_users_home_itself() {
    // A misconfigured AGENTMUX_DATA_HOME=$HOME must not chmod ~ to 0700.
    let tmp = TempDir::new().unwrap();
    let home = tmp.path().join("home");
    std::fs::create_dir(&home).unwrap();
    set_mode(&home, 0o755);
    assert_eq!(
        ensure_owner_only_dir_inner(&home, Some(&home)).unwrap(),
        OwnerOnlyOutcome::NotApplicable
    );
    assert_eq!(mode_of(&home), 0o755);
}

#[cfg(unix)]
#[test]
fn owner_only_refuses_a_file_where_the_root_should_be() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().join(".agentmux");
    std::fs::write(&root, b"not a dir").unwrap();
    assert!(ensure_owner_only_dir_inner(&root, None).is_err());
}

#[cfg(unix)]
#[test]
fn ensure_dirs_leaves_the_root_owner_only_and_still_creates_the_tree() {
    with_home_override(|root| {
        clear_channel_env();
        set_mode(root, 0o755); // an install from before #3682
        let paths = DataPaths::resolve("0.0.0", &RuntimeMode::Installed).unwrap();
        paths.ensure_dirs().unwrap();
        assert_eq!(mode_of(root), 0o700);
        for d in [&paths.data_dir, &paths.config_dir, &paths.agents_dir, &paths.shared_dir] {
            assert!(d.is_dir(), "{} not created", d.display());
        }
        assert!(paths.data_dir.join("db").is_dir());
    });
}

#[cfg(not(unix))]
#[test]
fn owner_only_is_not_applicable_off_unix() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().join(".agentmux");
    assert_eq!(ensure_owner_only_dir_inner(&root, None).unwrap(), OwnerOnlyOutcome::NotApplicable);
    // Nothing created either — ensure_dirs' own create_dir_all does that.
    assert!(!root.exists());
}

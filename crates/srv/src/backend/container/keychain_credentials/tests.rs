// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Tests of the Keychain login staging (moved out of `keychain_credentials.rs`, which has a size ceiling).

use super::*;
use std::cell::RefCell;
use std::collections::HashMap;

struct Fake {
    entries: RefCell<HashMap<String, String>>,
    touched: RefCell<Vec<String>>,
    unreadable: std::cell::Cell<bool>,
    /// The next write stores only the first half of the value and then fails, as `security -i` does with a
    /// line that is too long.
    cut_next_write: std::cell::Cell<bool>,
}
impl Fake {
    fn with(service: &str, raw: Option<String>) -> Self {
        let fake = Fake {
            entries: RefCell::new(HashMap::new()),
            touched: RefCell::new(Vec::new()),
            unreadable: std::cell::Cell::new(false),
            cut_next_write: std::cell::Cell::new(false),
        };
        if let Some(raw) = raw {
            fake.entries.borrow_mut().insert(service.to_string(), raw);
        }
        fake
    }
}
impl Keychain for Fake {
    fn read(&self, service: &str) -> Result<Option<Zeroizing<String>>, String> {
        self.touched.borrow_mut().push(format!("read {service}"));
        if self.unreadable.get() {
            return Err("locked".to_string());
        }
        Ok(self
            .entries
            .borrow()
            .get(service)
            .cloned()
            .map(Zeroizing::new))
    }
    fn write(&self, service: &str, value: &str) -> Result<(), String> {
        self.touched.borrow_mut().push(format!("write {service}"));
        if self.cut_next_write.replace(false) {
            self.entries
                .borrow_mut()
                .insert(service.to_string(), value[..value.len() / 2].to_string());
            return Err("security exited with 1".to_string());
        }
        self.entries
            .borrow_mut()
            .insert(service.to_string(), value.to_string());
        Ok(())
    }
}

const ACCOUNT: &str = "acct-1111";

/// The same JSON whatever order its keys were written in.
fn json(text: &str) -> serde_json::Value {
    serde_json::from_str(text).unwrap()
}

/// A login whose Keychain entry also keeps `extra` bytes of other data (MCP tokens, say).
fn big_entry(expires: i64, extra: usize) -> String {
    format!(
        r#"{{"claudeAiOauth":{{"accessToken":"a-big","refreshToken":"r-big","expiresAt":{expires}}},"mcpOAuth":{{"server":"{}"}}}}"#,
        "t".repeat(extra)
    )
}

fn login(expires: i64, tag: &str) -> String {
    format!(
        r#"{{"claudeAiOauth":{{"accessToken":"a-{tag}","refreshToken":"r-{tag}","expiresAt":{expires}}}}}"#
    )
}

/// A host config dir signed in as `ACCOUNT`, and an AgentMux root, both temporary.
fn setup() -> (tempfile::TempDir, String) {
    let tmp = tempfile::tempdir().unwrap();
    let config = tmp.path().join("account");
    std::fs::create_dir_all(&config).unwrap();
    std::fs::write(
        config.join(".claude.json"),
        format!(
            r#"{{"oauthAccount":{{"accountUuid":"{ACCOUNT}","emailAddress":"me@example.com"}}}}"#
        ),
    )
    .unwrap();
    let dir = config.to_string_lossy().into_owned();
    (tmp, dir)
}

fn run(
    kc: &Fake,
    tmp: &tempfile::TempDir,
    config: &str,
    container_account: Option<&str>,
) -> (Result<PathBuf, String>, Vec<Moved>) {
    let mut moved = Vec::new();
    let result = prepare(
        kc,
        &tmp.path().join("root"),
        "agentmux-c1",
        config,
        container_account,
        &mut |m| moved.push(m),
    );
    (result, moved)
}

#[test]
fn the_service_name_is_claudes_for_that_config_dir() {
    // A placeholder path: the name is a hash of the whole path, so any path has one.
    let dir =
        "/Users/example/.agentmux/shared/identities/00000000-0000-0000-0000-000000000000/claude";
    assert_eq!(service_name(dir), "Claude Code-credentials-c6fb88e5");
}

#[test]
fn the_first_start_stages_the_keychain_login_privately_and_only_reads_that_entry() {
    let (tmp, config) = setup();
    let service = service_name(&config);
    let kc = Fake::with(&service, Some(login(100, "kc")));
    let (result, moved) = run(&kc, &tmp, &config, None);
    let dir = result.unwrap();
    assert_eq!(
        std::fs::read_to_string(dir.join(".credentials.json")).unwrap(),
        login(100, "kc")
    );
    assert_eq!(
        *kc.touched.borrow(),
        vec![format!("read {service}")],
        "only the bound account's own entry is touched"
    );
    assert_eq!(
        moved,
        vec![Moved {
            direction: Direction::KeychainToContainer,
            account: "me@example.com".into(),
            outcome: Ok(())
        }]
    );
    assert!(
        dir.starts_with(tmp.path().join("root").join(STAGING_ROOT)) && !dir.starts_with(&config)
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&dir), 0o700);
        assert_eq!(mode(&dir.join(".credentials.json")), 0o600);
    }
}

#[test]
fn no_login_in_the_keychain_stages_nothing_and_says_so() {
    let (tmp, config) = setup();
    let kc = Fake::with(&service_name(&config), None);
    let (result, moved) = run(&kc, &tmp, &config, None);
    assert!(result.unwrap_err().contains("no login in the Keychain"));
    assert!(moved.is_empty());
    assert!(!staging_dir(&tmp.path().join("root"), "agentmux-c1", &config).exists());
    let kc = Fake::with(&service_name(&config), Some("not a login".into()));
    assert!(run(&kc, &tmp, &config, None).0.is_err());
}

#[test]
fn a_login_the_host_refreshed_reaches_the_staged_file_in_place() {
    let (tmp, config) = setup();
    let service = service_name(&config);
    let kc = Fake::with(&service, Some(login(100, "old")));
    let dir = run(&kc, &tmp, &config, None).0.unwrap();
    let file = dir.join(".credentials.json");
    #[cfg(unix)]
    let inode = std::os::unix::fs::MetadataExt::ino(&std::fs::metadata(&file).unwrap());
    kc.entries.borrow_mut().insert(service, login(200, "new"));
    let (result, _) = run(&kc, &tmp, &config, Some(ACCOUNT));
    assert_eq!(result.unwrap(), dir);
    assert_eq!(std::fs::read_to_string(&file).unwrap(), login(200, "new"));
    #[cfg(unix)]
    assert_eq!(
        std::os::unix::fs::MetadataExt::ino(&std::fs::metadata(&file).unwrap()),
        inode,
        "the mounted file stays the same file"
    );
}

#[test]
fn a_login_the_container_refreshed_is_written_back_for_the_same_account() {
    let (tmp, config) = setup();
    let service = service_name(&config);
    let kc = Fake::with(&service, Some(login(100, "old")));
    let dir = run(&kc, &tmp, &config, None).0.unwrap();
    std::fs::write(dir.join(".credentials.json"), login(300, "container")).unwrap();
    let (_, moved) = run(&kc, &tmp, &config, Some(ACCOUNT));
    assert_eq!(
        json(&kc.entries.borrow()[&service]),
        json(&login(300, "container"))
    );
    assert_eq!(
        moved,
        vec![Moved {
            direction: Direction::ContainerToKeychain,
            account: "me@example.com".into(),
            outcome: Ok(())
        }]
    );
    assert_eq!(
        kc.touched
            .borrow()
            .iter()
            .filter(|t| t.starts_with("write"))
            .count(),
        1
    );
}

#[test]
fn a_login_for_another_account_or_an_unknown_one_is_not_written_back() {
    for container_account in [Some("someone-else"), None] {
        let (tmp, config) = setup();
        let service = service_name(&config);
        let kc = Fake::with(&service, Some(login(100, "old")));
        let dir = run(&kc, &tmp, &config, None).0.unwrap();
        std::fs::write(dir.join(".credentials.json"), login(300, "other")).unwrap();
        let (_, moved) = run(&kc, &tmp, &config, container_account);
        assert_eq!(
            kc.entries.borrow()[&service],
            login(100, "old"),
            "the Keychain entry is untouched"
        );
        assert!(!kc.touched.borrow().iter().any(|t| t.starts_with("write")));
        assert!(
            matches!(
                moved.as_slice(),
                [Moved {
                    direction: Direction::ContainerToKeychain,
                    outcome: Err(_),
                    ..
                }]
            ),
            "{moved:?}"
        );
    }
}

#[test]
fn a_staged_login_that_is_older_than_the_keychains_is_replaced_not_written_back() {
    let (tmp, config) = setup();
    let service = service_name(&config);
    let kc = Fake::with(&service, Some(login(500, "kc")));
    let dir = run(&kc, &tmp, &config, None).0.unwrap();
    std::fs::write(dir.join(".credentials.json"), login(400, "stale")).unwrap();
    let _ = run(&kc, &tmp, &config, Some(ACCOUNT));
    assert_eq!(kc.entries.borrow()[&service], login(500, "kc"));
    assert_eq!(
        std::fs::read_to_string(dir.join(".credentials.json")).unwrap(),
        login(500, "kc")
    );
    assert!(!kc.touched.borrow().iter().any(|t| t.starts_with("write")));
}

#[test]
fn a_login_that_expires_implausibly_far_ahead_is_not_written_back_and_not_kept_as_pending() {
    let (tmp, config) = setup();
    let service = service_name(&config);
    let kc = Fake::with(&service, Some(login(100, "old")));
    let dir = run(&kc, &tmp, &config, None).0.unwrap();
    std::fs::write(dir.join(".credentials.json"), login(i64::MAX, "forged")).unwrap();
    let (_, moved) = run(&kc, &tmp, &config, Some(ACCOUNT));
    assert_eq!(kc.entries.borrow()[&service], login(100, "old"));
    assert!(!kc.touched.borrow().iter().any(|t| t.starts_with("write")));
    assert!(
        matches!(
            moved.as_slice(),
            [Moved {
                direction: Direction::ContainerToKeychain,
                outcome: Err(_),
                ..
            }]
        ),
        "{moved:?}"
    );
    // It must not hold the staging back from deletion either.
    assert!(!refresh_pending(
        &kc,
        &tmp.path().join("root"),
        "agentmux-c1",
        &config
    ));
}

#[test]
fn a_write_that_leaves_a_cut_off_entry_is_undone() {
    let (tmp, config) = setup();
    let service = service_name(&config);
    let kc = Fake::with(&service, Some(login(100, "old")));
    let dir = run(&kc, &tmp, &config, None).0.unwrap();
    std::fs::write(dir.join(".credentials.json"), login(300, "new")).unwrap();
    kc.cut_next_write.set(true);
    let (_, moved) = run(&kc, &tmp, &config, Some(ACCOUNT));
    assert_eq!(
        kc.entries.borrow()[&service],
        login(100, "old"),
        "the host's login is back as it was"
    );
    assert!(
        matches!(
            moved.as_slice(),
            [Moved {
                direction: Direction::ContainerToKeychain,
                outcome: Err(_),
                ..
            }]
        ),
        "{moved:?}"
    );
    // The refresh is still staged, so nothing is lost: it is tried again at the next start.
    assert_eq!(
        std::fs::read_to_string(dir.join(".credentials.json")).unwrap(),
        login(300, "new")
    );
}

#[cfg(target_os = "macos")]
#[test]
fn a_small_entry_goes_through_stdin_and_a_large_one_through_the_argument() {
    let small = write_call("svc", "me", &"x".repeat(1000));
    assert_eq!(small.args, ["-i"]);
    assert!(small
        .stdin
        .as_ref()
        .is_some_and(|l| l.len() <= MAX_SECURITY_LINE));
    assert!(
        !small.args.iter().any(|a| a.contains("78787878")),
        "not in the arguments"
    );
    let large = write_call("svc", "me", &"x".repeat(9000));
    assert!(large.stdin.is_none());
    assert_eq!(large.args[0], "add-generic-password");
    assert_eq!(large.args.last().unwrap().len(), 2 * 9000);
}

#[test]
fn a_refresh_is_written_back_whole_when_the_entry_is_over_8_kb() {
    let (tmp, config) = setup();
    let service = service_name(&config);
    let kc = Fake::with(&service, Some(big_entry(100, 9_000)));
    let dir = run(&kc, &tmp, &config, None).0.unwrap();
    std::fs::write(dir.join(".credentials.json"), login(300, "fresh")).unwrap();
    let (_, moved) = run(&kc, &tmp, &config, Some(ACCOUNT));
    let stored = json(&kc.entries.borrow()[&service]);
    assert_eq!(stored["claudeAiOauth"]["accessToken"], "a-fresh");
    assert_eq!(
        stored["mcpOAuth"]["server"].as_str().unwrap().len(),
        9_000,
        "the rest of the entry is kept"
    );
    assert!(matches!(
        moved.as_slice(),
        [Moved {
            direction: Direction::ContainerToKeychain,
            outcome: Ok(()),
            ..
        }]
    ));
}

#[test]
fn a_host_account_with_no_known_id_never_gets_a_write_back() {
    let (tmp, config) = setup();
    std::fs::remove_file(Path::new(&config).join(".claude.json")).unwrap();
    let service = service_name(&config);
    let kc = Fake::with(&service, Some(login(100, "old")));
    let dir = run(&kc, &tmp, &config, None).0.unwrap();
    std::fs::write(dir.join(".credentials.json"), login(300, "c")).unwrap();
    let (_, moved) = run(&kc, &tmp, &config, Some("")); // an empty id never matches
    assert!(!kc.touched.borrow().iter().any(|t| t.starts_with("write")));
    assert!(matches!(
        moved.as_slice(),
        [Moved {
            outcome: Err(_),
            ..
        }]
    ));
}

#[test]
fn a_signed_out_keychain_entry_is_not_created_again_from_the_staged_login() {
    let (tmp, config) = setup();
    let service = service_name(&config);
    let kc = Fake::with(&service, Some(login(100, "old")));
    let dir = run(&kc, &tmp, &config, None).0.unwrap();
    std::fs::write(dir.join(".credentials.json"), login(300, "c")).unwrap();
    kc.entries.borrow_mut().clear();
    assert_eq!(run(&kc, &tmp, &config, Some(ACCOUNT)).0.unwrap(), dir);
    assert!(!kc.touched.borrow().iter().any(|t| t.starts_with("write")));
}

#[test]
fn removing_the_container_deletes_the_staged_login_and_its_note() {
    let (tmp, config) = setup();
    let kc = Fake::with(&service_name(&config), Some(login(100, "kc")));
    let root = tmp.path().join("root");
    run(&kc, &tmp, &config, None).0.unwrap();
    assert_eq!(
        staged_config_dirs(&root, "agentmux-c1"),
        vec![config.clone()]
    );
    remove_staging(&root, "agentmux-c1", &config);
    assert!(!staging_dir(&root, "agentmux-c1", &config).exists());
    assert!(staged_config_dirs(&root, "agentmux-c1").is_empty());
    assert!(
        !root.join(STAGING_ROOT).join("agentmux-c1").exists(),
        "the container's folder goes with its last login"
    );
    remove_staging(&root, "agentmux-c1", &config); // already gone: fine
}

/// Against the real login keychain, with an entry of its own that this test creates and deletes (never a
/// Claude entry): `cargo test -p agentmux-srv -- --ignored keychain_credentials_live`
#[cfg(target_os = "macos")]
#[test]
#[ignore = "uses the real login keychain"]
fn keychain_credentials_live_round_trip_through_security_cli() {
    let service = format!(
        "agentmux-test-container-login-{}",
        uuid::Uuid::new_v4().simple()
    );
    let user = std::env::var("USER").unwrap();
    let first = login(100, "first");
    let added = std::process::Command::new("/usr/bin/security")
        .args([
            "add-generic-password",
            "-s",
            &service,
            "-a",
            &user,
            "-w",
            &first,
        ])
        .status()
        .unwrap();
    assert!(added.success());
    let keychain = SecurityCli;
    let read = keychain.read(&service).map(|v| v.map(|z| z.to_string()));
    let second = login(200, "second \"quoted\" \\ and spaces");
    let written = keychain.write(&service, &second);
    let read_again = keychain.read(&service).map(|v| v.map(|z| z.to_string()));
    let missing = keychain
        .read(&format!("{service}-absent"))
        .map(|v| v.map(|z| z.to_string()));
    let _ = std::process::Command::new("/usr/bin/security")
        .args(["delete-generic-password", "-s", &service, "-a", &user])
        .status();
    assert_eq!(read, Ok(Some(first)));
    assert_eq!(written, Ok(()));
    assert_eq!(
        read_again,
        Ok(Some(second)),
        "an update keeps the text exactly"
    );
    assert_eq!(missing, Ok(None));
}

/// An entry past what one `security -i` line takes (many MCP tokens) is written whole, through the argument form.
#[cfg(target_os = "macos")]
#[test]
#[ignore = "uses the real login keychain"]
fn keychain_credentials_live_writes_an_entry_over_8_kb() {
    let service = format!(
        "agentmux-test-container-login-{}",
        uuid::Uuid::new_v4().simple()
    );
    let user = std::env::var("USER").unwrap();
    let added = std::process::Command::new("/usr/bin/security")
        .args([
            "add-generic-password",
            "-s",
            &service,
            "-a",
            &user,
            "-w",
            &login(100, "first"),
        ])
        .status()
        .unwrap();
    assert!(added.success());
    let big = big_entry(300, 12_000);
    let keychain = SecurityCli;
    let written = keychain.write(&service, &big);
    let read = keychain.read(&service).map(|v| v.map(|z| z.to_string()));
    let _ = std::process::Command::new("/usr/bin/security")
        .args(["delete-generic-password", "-s", &service, "-a", &user])
        .status();
    assert_eq!(written, Ok(()));
    assert_eq!(read, Ok(Some(big)), "all of it is stored, none cut off");
}

/// The whole flow against the real login keychain, for a throwaway config dir (so a throwaway service name):
/// stage, a refresh in the container written back, a refresh on the host brought in.
#[cfg(target_os = "macos")]
#[test]
#[ignore = "uses the real login keychain"]
fn keychain_credentials_live_prepare_flow() {
    let (tmp, config) = setup();
    let service = service_name(&config);
    let user = std::env::var("USER").unwrap();
    let security = |args: &[&str]| {
        std::process::Command::new("/usr/bin/security")
            .args(args)
            .status()
            .unwrap()
            .success()
    };
    assert!(security(&[
        "add-generic-password",
        "-s",
        &service,
        "-a",
        &user,
        "-w",
        &login(100, "host")
    ]));
    let root = tmp.path().join("root");
    let mut moved = Vec::new();
    let outcome = (|| -> Result<(), String> {
        let dir = prepare(
            &SecurityCli,
            &root,
            "agentmux-live",
            &config,
            None,
            &mut |m| moved.push(m),
        )?;
        let file = dir.join(".credentials.json");
        assert_eq!(std::fs::read_to_string(&file).unwrap(), login(100, "host"));
        std::fs::write(&file, login(300, "container")).unwrap();
        prepare(
            &SecurityCli,
            &root,
            "agentmux-live",
            &config,
            Some(ACCOUNT),
            &mut |m| moved.push(m),
        )?;
        assert_eq!(
            SecurityCli.read(&service)?.map(|z| json(&z)),
            Some(json(&login(300, "container")))
        );
        assert!(security(&[
            "add-generic-password",
            "-U",
            "-s",
            &service,
            "-a",
            &user,
            "-w",
            &login(500, "host-again")
        ]));
        prepare(
            &SecurityCli,
            &root,
            "agentmux-live",
            &config,
            Some(ACCOUNT),
            &mut |m| moved.push(m),
        )?;
        assert_eq!(
            std::fs::read_to_string(&file).unwrap(),
            login(500, "host-again")
        );
        Ok(())
    })();
    let _ = security(&["delete-generic-password", "-s", &service, "-a", &user]);
    outcome.unwrap();
    let directions: Vec<_> = moved
        .iter()
        .map(|m| (m.direction, m.outcome.is_ok()))
        .collect();
    assert_eq!(
        directions,
        vec![
            (Direction::KeychainToContainer, true),
            (Direction::ContainerToKeychain, true),
            (Direction::KeychainToContainer, true)
        ]
    );
}

#[test]
fn the_containers_account_is_read_from_the_archive_docker_returns() {
    let tar_of = |name: &str, body: &str| {
        let mut builder = tar::Builder::new(Vec::new());
        let mut header = tar::Header::new_gnu();
        header.set_size(body.len() as u64);
        header.set_mode(0o600);
        header.set_cksum();
        builder
            .append_data(&mut header, name, body.as_bytes())
            .unwrap();
        builder.into_inner().unwrap()
    };
    let with = tar_of(
        ".claude.json",
        r#"{"oauthAccount":{"accountUuid":"acct-1111"},"other":1}"#,
    );
    assert_eq!(account_in_archive(&with).as_deref(), Some("acct-1111"));
    assert_eq!(account_in_archive(&tar_of(".claude.json", "{}")), None);
    assert_eq!(
        account_in_archive(&tar_of(".claude.json", "not json")),
        None
    );
    assert_eq!(account_in_archive(b""), None);
}

#[test]
fn an_unreadable_keychain_keeps_the_staged_login_and_deletes_nothing() {
    let (tmp, config) = setup();
    let kc = Fake::with(&service_name(&config), Some(login(100, "old")));
    let dir = run(&kc, &tmp, &config, None).0.unwrap();
    std::fs::write(dir.join(".credentials.json"), login(300, "container")).unwrap();
    kc.unreadable.set(true);
    let (result, moved) = run(&kc, &tmp, &config, Some(ACCOUNT));
    assert_eq!(
        result.unwrap(),
        dir,
        "the mount does not change under a running agent"
    );
    assert_eq!(
        std::fs::read_to_string(dir.join(".credentials.json")).unwrap(),
        login(300, "container")
    );
    assert!(
        matches!(
            moved.as_slice(),
            [Moved {
                outcome: Err(_),
                ..
            }]
        ),
        "{moved:?}"
    );
    assert!(!kc.touched.borrow().iter().any(|t| t.starts_with("write")));
    // With nothing staged, an unreadable Keychain is an error the pane can show.
    let (tmp2, config2) = setup();
    let kc2 = Fake::with(&service_name(&config2), Some(login(100, "x")));
    kc2.unreadable.set(true);
    assert!(run(&kc2, &tmp2, &config2, None).0.is_err());
}

#[test]
fn a_staged_refresh_the_keychain_lacks_is_pending_and_is_not_deleted_with_the_container() {
    let (tmp, config) = setup();
    let root = tmp.path().join("root");
    let service = service_name(&config);
    let kc = Fake::with(&service, Some(login(100, "old")));
    assert!(
        !refresh_pending(&kc, &root, "agentmux-c1", &config),
        "nothing staged"
    );
    let dir = run(&kc, &tmp, &config, None).0.unwrap();
    assert!(
        !refresh_pending(&kc, &root, "agentmux-c1", &config),
        "staged equals the Keychain's"
    );
    std::fs::write(dir.join(".credentials.json"), login(300, "container")).unwrap();
    assert!(
        refresh_pending(&kc, &root, "agentmux-c1", &config),
        "the container refreshed and nothing was written back"
    );
    kc.unreadable.set(true);
    assert!(
        refresh_pending(&kc, &root, "agentmux-c1", &config),
        "can't tell: keep it"
    );
    kc.unreadable.set(false);
    kc.entries
        .borrow_mut()
        .insert(service, login(300, "container"));
    assert!(
        !refresh_pending(&kc, &root, "agentmux-c1", &config),
        "written back: safe to delete"
    );
}

#[test]
fn rebinding_the_agent_to_another_account_never_mounts_the_old_accounts_login() {
    let (tmp, config_a) = setup();
    let config_b = tmp.path().join("account-b");
    std::fs::create_dir_all(&config_b).unwrap();
    std::fs::write(
        config_b.join(".claude.json"),
        r#"{"oauthAccount":{"accountUuid":"acct-2222","emailAddress":"b@example.com"}}"#,
    )
    .unwrap();
    let config_b = config_b.to_string_lossy().into_owned();
    let root = tmp.path().join("root");
    let (service_a, service_b) = (service_name(&config_a), service_name(&config_b));
    let kc = Fake::with(&service_a, Some(login(100, "a")));
    let dir_a = run(&kc, &tmp, &config_a, None).0.unwrap();
    std::fs::write(dir_a.join(".credentials.json"), login(300, "a-refreshed")).unwrap();
    assert_ne!(
        staging_dir(&root, "agentmux-c1", &config_a),
        staging_dir(&root, "agentmux-c1", &config_b),
        "another account mounts another path, so the container is recreated"
    );

    // B has no login yet: nothing of A's is offered for it, and A's refresh went back to A's entry.
    let (result, _) = run(&kc, &tmp, &config_b, Some(ACCOUNT));
    assert!(
        result.is_err(),
        "B has no login and A's is not offered in its place"
    );
    assert_eq!(
        json(&kc.entries.borrow()[&service_a]),
        json(&login(300, "a-refreshed"))
    );
    assert!(
        !dir_a.exists(),
        "A's refresh is in the Keychain, so its staged login is deleted"
    );

    // B gets a login: the staged file is B's, at B's own path.
    kc.entries.borrow_mut().insert(service_b, login(50, "b"));
    let dir_b = run(&kc, &tmp, &config_b, None).0.unwrap();
    assert_ne!(dir_b, dir_a);
    assert_eq!(
        std::fs::read_to_string(dir_b.join(".credentials.json")).unwrap(),
        login(50, "b")
    );
    assert_eq!(staged_config_dirs(&root, "agentmux-c1"), vec![config_b]);
}

#[test]
fn an_old_accounts_pending_refresh_is_kept_when_the_agent_is_rebound() {
    let (tmp, config_a) = setup();
    let config_b = tmp.path().join("account-b").to_string_lossy().into_owned();
    std::fs::create_dir_all(&config_b).unwrap();
    let service_a = service_name(&config_a);
    let kc = Fake::with(&service_a, Some(login(100, "a")));
    kc.entries
        .borrow_mut()
        .insert(service_name(&config_b), login(50, "b"));
    let dir_a = run(&kc, &tmp, &config_a, None).0.unwrap();
    std::fs::write(dir_a.join(".credentials.json"), login(300, "a-refreshed")).unwrap();
    // The container's account can't be told, so A's refresh can't be written back yet.
    let dir_b = run(&kc, &tmp, &config_b, None).0.unwrap();
    assert_ne!(dir_a, dir_b);
    assert_eq!(
        kc.entries.borrow()[&service_a],
        login(100, "a"),
        "not written back"
    );
    assert_eq!(
        std::fs::read_to_string(dir_a.join(".credentials.json")).unwrap(),
        login(300, "a-refreshed"),
        "kept, not deleted"
    );
}

#[test]
fn syncing_again_while_two_accounts_are_staged_terminates_and_keeps_the_pending_one() {
    let (tmp, config_a) = setup();
    let config_b = tmp.path().join("account-b").to_string_lossy().into_owned();
    std::fs::create_dir_all(&config_b).unwrap();
    let kc = Fake::with(&service_name(&config_a), Some(login(100, "a")));
    kc.entries
        .borrow_mut()
        .insert(service_name(&config_b), login(50, "b"));
    let dir_a = run(&kc, &tmp, &config_a, None).0.unwrap();
    std::fs::write(dir_a.join(".credentials.json"), login(300, "a-refreshed")).unwrap();
    let dir_b = run(&kc, &tmp, &config_b, None).0.unwrap();
    let root = tmp.path().join("root");
    assert_eq!(
        staged_config_dirs(&root, "agentmux-c1").len(),
        2,
        "A stays staged, B is mounted"
    );
    // Every turn start, and the sync before a removal, run again over both.
    for _ in 0..3 {
        assert_eq!(run(&kc, &tmp, &config_b, None).0.unwrap(), dir_b);
        let mut moved = Vec::new();
        for dir in staged_config_dirs(&root, "agentmux-c1") {
            let _ = sync_account(&kc, &root, "agentmux-c1", &dir, None, &mut |m| {
                moved.push(m)
            });
        }
    }
    assert_eq!(staged_config_dirs(&root, "agentmux-c1").len(), 2);
    assert_eq!(
        std::fs::read_to_string(dir_a.join(".credentials.json")).unwrap(),
        login(300, "a-refreshed")
    );
}

#[test]
fn a_write_back_replaces_only_the_login_and_keeps_the_rest_of_the_entry() {
    let (tmp, config) = setup();
    let service = service_name(&config);
    let with_extra = r#"{"claudeAiOauth":{"accessToken":"a","refreshToken":"r","expiresAt":100},"mcpOAuth":{"server":{"token":"keep"}}}"#;
    let kc = Fake::with(&service, Some(with_extra.to_string()));
    let dir = run(&kc, &tmp, &config, None).0.unwrap();
    std::fs::write(dir.join(".credentials.json"), login(300, "container")).unwrap();
    let _ = run(&kc, &tmp, &config, Some(ACCOUNT));
    let written: serde_json::Value = serde_json::from_str(&kc.entries.borrow()[&service]).unwrap();
    assert_eq!(written["claudeAiOauth"]["accessToken"], "a-container");
    assert_eq!(written["claudeAiOauth"]["expiresAt"], 300);
    assert_eq!(written["mcpOAuth"]["server"]["token"], "keep");
    assert_eq!(with_login_of("not json", &login(1, "x")), None);
    assert_eq!(
        with_login_of(with_extra, "{}"),
        None,
        "a staged file with no login is never written"
    );
}

#[test]
fn a_container_with_no_login_says_why_and_how_to_sign_in() {
    let msg = crate::backend::container::ContainerError::NoLogin(
        "this account has no login in the Keychain; sign in with it first".into(),
    )
    .user_message();
    assert!(
        msg.contains("no login in the Keychain")
            && msg.contains("Connectors > Accounts")
            && msg.contains("API key"),
        "{msg}"
    );
    assert!(!msg.contains("Docker"), "it is not a Docker problem: {msg}");
}

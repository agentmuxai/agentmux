// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::*;
use crate::backend::mcp_config_file::with_data_home;
use serde_json::json;
use std::path::Path;

fn none(_: &str) -> Option<String> {
    None
}

fn as_env(entries: Vec<(String, String)>) -> HashMap<String, String> {
    entries.into_iter().collect()
}

/// The (key, value) pairs the `GIT_CONFIG_*` entries of `env` name, in order (a slot the process environment fills
/// is not in `env`).
fn pairs(env: &HashMap<String, String>) -> Vec<(String, String)> {
    let count: usize = env.get("GIT_CONFIG_COUNT").and_then(|c| c.parse().ok()).unwrap_or(0);
    (0..count).filter_map(|i| Some((env.get(&format!("GIT_CONFIG_KEY_{i}"))?.clone(), env.get(&format!("GIT_CONFIG_VALUE_{i}"))?.clone()))).collect()
}

fn value_of(env: &HashMap<String, String>, key: &str) -> Option<String> {
    pairs(env).into_iter().rev().find(|(k, _)| k.eq_ignore_ascii_case(key)).map(|(_, v)| v)
}

fn pane(mode: &str, provider: &str, cwd: &Path) -> crate::backend::obj::MetaMapType {
    [("agentMode", json!(mode)), ("agentProvider", json!(provider)), ("cmd:cwd", json!(cwd.to_string_lossy()))].into_iter().map(|(k, v)| (k.to_string(), v)).collect()
}

#[test]
fn every_program_setting_is_given_a_value_that_runs_nothing_of_the_folders() {
    let env = as_env(entries_over(none, &HashMap::new()));
    for (key, value) in SETTINGS {
        assert_eq!(value_of(&env, key).as_deref(), Some(*value), "{key}");
    }
    assert_eq!(value_of(&env, "credential.helper").as_deref(), Some(""), "the helpers before it are cleared");
    assert_eq!(env.get("GIT_PROXY_COMMAND").map(String::as_str), Some(""));
}

#[test]
fn what_the_environment_already_carries_is_kept_beside_them() {
    let carried = HashMap::from([
        ("GIT_CONFIG_COUNT".to_string(), "3".to_string()),
        ("GIT_CONFIG_KEY_0".to_string(), "core.hooksPath".to_string()),
        ("GIT_CONFIG_VALUE_0".to_string(), "/hooks".to_string()),
        ("GIT_CONFIG_KEY_1".to_string(), "credential.helper".to_string()),
        ("GIT_CONFIG_VALUE_1".to_string(), String::new()),
        ("GIT_CONFIG_KEY_2".to_string(), "credential.helper".to_string()),
        ("GIT_CONFIG_VALUE_2".to_string(), "broker".to_string()),
        ("GIT_PROXY_COMMAND".to_string(), "the-persons-proxy".to_string()),
    ]);
    let mut env = carried.clone();
    env.extend(entries_over(|k| carried.get(k).cloned(), &HashMap::new()));
    let all = pairs(&env);
    assert_eq!(&all[..3], &[("core.hooksPath".into(), "/hooks".into()), ("credential.helper".into(), String::new()), ("credential.helper".into(), "broker".into())]);
    assert_eq!(all.iter().filter(|(k, _)| k == "credential.helper").count(), 2, "a list that is already there is not added to");
    assert_eq!(all.len(), 3 + SETTINGS.len() + PAGED_COMMANDS.len());
    assert_eq!(env.get("GIT_PROXY_COMMAND").map(String::as_str), Some("the-persons-proxy"));
    let mut own = carried.clone();
    own.insert("GIT_CONFIG_KEY_1".into(), "core.editor".into());
    own.insert("GIT_CONFIG_VALUE_1".into(), "vim".into());
    own.extend(entries_over(|k| own.get(k).cloned(), &HashMap::new()));
    assert_eq!(value_of(&own, "core.editor").as_deref(), Some("true"), "over the agent's own value");
}

#[test]
fn only_a_host_agent_in_such_a_folder_is_given_them_whatever_its_harness() {
    let (data, folder, other) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    with_data_home(data.path(), || {
        crate::backend::mcp_folder_rule::mark(&folder.path().to_string_lossy());
        let env_of = |meta: &crate::backend::obj::MetaMapType| {
            let mut env = HashMap::from([("KEPT".to_string(), "yes".to_string())]);
            apply_for_spawn(&mut env, meta);
            env
        };
        for provider in ["claude", "codex", "gemini"] {
            let there = env_of(&pane("host", provider, folder.path()));
            assert_eq!(value_of(&there, "core.fsmonitor").as_deref(), Some("false"), "{provider}");
            assert_eq!(there.get("KEPT").map(String::as_str), Some("yes"));
            assert_eq!(there.get(GIT_NO_EXT_DIFF_ENV).map(String::as_str), Some("1"), "{provider}");
            assert_eq!(there.get(GIT_FUNCTION_NAME).map(String::as_str), Some(GIT_FLAGS_FUNCTION_BODY), "{provider}: the diff step, where there is no other git function");
        }
        for (why, meta) in [
            ("a folder no container agent has to do with", pane("host", "claude", other.path())),
            ("the container agent itself", pane("container", "claude", folder.path())),
        ] {
            assert_eq!(env_of(&meta), HashMap::from([("KEPT".to_string(), "yes".to_string())]), "{why}: git left as it is");
        }
    });
}

/// Runs git in `repo` with none of the person's own configuration, and `env` on top.
fn run_git(repo: &Path, home: &Path, args: &[&str], env: &HashMap<String, String>) -> std::process::Output {
    run_git_with(repo, home, args, env, "")
}

/// [`run_git`] with `input` on git's standard input.
fn run_git_with(repo: &Path, home: &Path, args: &[&str], env: &HashMap<String, String>, input: &str) -> std::process::Output {
    run_program("git", repo, home, args, env, input)
}

fn run_program(program: &str, repo: &Path, home: &Path, args: &[&str], env: &HashMap<String, String>, input: &str) -> std::process::Output {
    use std::io::Write;
    use agentmux_common::win32::NoWindow;
    let mut cmd = std::process::Command::new(program);
    cmd.args(args)
        .current_dir(repo)
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env_remove("GIT_CONFIG_COUNT")
        .env_remove("GIT_PROXY_COMMAND")
        .env_remove("GIT_SSH_COMMAND")
        .env_remove("GIT_EXTERNAL_DIFF")
        .envs(env)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    cmd.no_window();
    let mut child = cmd.spawn().expect("git runs");
    let _ = child.stdin.take().expect("piped").write_all(input.as_bytes());
    child.wait_with_output().expect("git ends")
}

/// A repository whose own `.git/config` names a marker program for every setting that runs one, and a remote
/// that names a command (`ext::`), one that goes through the proxy and one through ssh.
struct Folder {
    dir: tempfile::TempDir,
    marks: std::path::PathBuf,
}

impl Folder {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let marks = dir.path().join("marks");
        std::fs::create_dir_all(&marks).unwrap();
        let script = dir.path().join("mark.sh");
        std::fs::write(&script, format!("#!/bin/sh\necho \"$@\" >> '{}/'\"$1\"\nexit 1\n", slashes(&marks))).unwrap();
        let folder = Folder { dir, marks };
        let (repo, remote) = (folder.repo(), folder.dir.path().join("remote.git"));
        for args in [vec!["init", "-q", "."], vec!["config", "user.email", "t@example.invalid"], vec!["config", "user.name", "t"]] {
            folder.git(&args, &HashMap::new());
        }
        std::fs::write(repo.join("a.txt"), "one\n").unwrap();
        folder.git(&["add", "a.txt"], &HashMap::new());
        folder.git(&["commit", "-q", "-m", "one"], &HashMap::new());
        folder.git(&["init", "-q", "--bare", &slashes(&remote)], &HashMap::new());
        let mark = |name: &str| format!("sh {} {name}", slashes(&script));
        for (key, value) in [
            ("core.fsmonitor", mark("fsmonitor")),
            ("core.sshCommand", mark("ssh")),
            ("core.gitProxy", mark("proxy")),
            ("core.pager", mark("pager")),
            ("pager.log", mark("pagerlog")),
            ("pager.status", mark("pagerstatus")),
            ("core.editor", mark("editor")),
            ("core.askPass", mark("askpass")),
            ("gpg.program", mark("gpg")),
            ("gpg.ssh.program", mark("gpgssh")),
            ("credential.helper", format!("!{}", mark("credential"))),
            ("sequence.editor", mark("sequence")),
            ("protocol.ext.allow", "always".to_string()),
            ("remote.origin.url", slashes(&remote)),
            ("remote.viaext.url", format!("ext::sh {} ext %S", slashes(&script))),
            ("remote.viassh.url", "ssh://127.0.0.1:1/x".to_string()),
            ("remote.viagit.url", "git://127.0.0.1:9/x".to_string()),
            ("remotes.grp", "viaup".to_string()),
            ("remote.viaup.url", slashes(&remote)),
            ("remote.viaup.uploadpack", mark("uploadpack")),
            ("remote.viaup.receivepack", mark("receivepack")),
        ] {
            folder.git(&["config", key, &value], &HashMap::new());
        }
        folder
    }

    fn repo(&self) -> std::path::PathBuf {
        self.dir.path().join("repo")
    }

    /// Runs git in the folder; nothing of the person's own configuration is read. What it printed.
    fn git(&self, args: &[&str], env: &HashMap<String, String>) -> String {
        let repo = self.repo();
        std::fs::create_dir_all(&repo).unwrap();
        let out = run_git(&repo, self.dir.path(), args, env);
        format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr))
    }

    /// The marker programs that ran, by name.
    fn ran(&self) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(&self.marks).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        names.sort();
        names
    }

    fn forget(&self) {
        for entry in std::fs::read_dir(&self.marks).unwrap().flatten() {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

fn slashes(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

#[test]
fn git_in_the_folder_runs_nothing_its_own_config_names() {
    let folder = Folder::new();
    folder.forget();
    let given = as_env(entries_over(none, &HashMap::new()));
    // Without the settings the folder's config does run its programs: the check is not empty.
    for args in [vec!["status"], vec!["fetch", "viaext"], vec!["ls-remote", "viassh"]] {
        folder.git(&args, &HashMap::new());
    }
    assert_eq!(folder.ran(), vec!["ext", "fsmonitor", "ssh"], "what the folder's config runs by itself");
    let proxied = folder.git(&["ls-remote", "viagit"], &HashMap::new());
    assert!(proxied.contains("cannot start proxy"), "and the proxy command it names is started: {proxied}");
    folder.forget();
    for args in [
        vec!["status"],
        vec!["log"],
        vec!["diff"],
        vec!["fetch", "origin"],
        vec!["fetch", "viaext"],
        vec!["ls-remote", "viassh"],
    ] {
        folder.git(&args, &given);
    }
    assert_eq!(folder.ran(), Vec::<String>::new(), "with them, none of it runs");
    let direct = folder.git(&["ls-remote", "viagit"], &given);
    assert!(!direct.contains("proxy"), "no proxy command is started: {direct}");
    // And git still works there: the history and the file remote are read.
    let out = run_git(&folder.repo(), folder.dir.path(), &["log", "--oneline"], &given);
    assert!(String::from_utf8_lossy(&out.stdout).contains("one"), "{out:?}");
}

#[test]
fn the_settings_are_what_git_reads_for_the_programs_a_person_is_not_present_for() {
    let folder = Folder::new();
    let given = as_env(entries_over(none, &HashMap::new()));
    let read = |args: &[&str]| String::from_utf8_lossy(&run_git(&folder.repo(), folder.dir.path(), args, &given).stdout).trim().to_string();
    assert_eq!(read(&["var", "GIT_PAGER"]), "cat", "the pager");
    assert_eq!(read(&["var", "GIT_EDITOR"]), "true", "the editor");
    for (key, value) in SETTINGS {
        assert_eq!(read(&["config", "--get", key]), *value, "{key}");
    }
    let helpers = run_git(&folder.repo(), folder.dir.path(), &["config", "--get-all", "credential.helper"], &given);
    let helpers = String::from_utf8_lossy(&helpers.stdout).replace("\r\n", "\n");
    assert_eq!(helpers.strip_suffix('\n').unwrap_or(&helpers).rsplit('\n').next(), Some(""), "the last helper is the empty one, which clears those before it: {helpers:?}");
}

#[test]
fn the_persons_own_values_are_read_from_the_system_and_global_configuration_alone() {
    let scoped = "global\tcore.sshcommand ssh -i C:/keys/id -o IdentitiesOnly=yes\nsystem\tcore.editor nano\nglobal\tcore.editor code --wait\nlocal\tgpg.program /folder/evil\ncommand\tgpg.ssh.program /env/evil\nglobal\tgpg.program gpg2\n";
    let own = parse_scoped(scoped);
    assert_eq!(own.get("core.sshcommand").map(String::as_str), Some("ssh -i C:/keys/id -o IdentitiesOnly=yes"));
    assert_eq!(own.get("core.editor").map(String::as_str), Some("code --wait"), "global over system");
    assert_eq!(own.get("gpg.program").map(String::as_str), Some("gpg2"), "and a folder's or the environment's value is not the person's");
    assert!(!own.contains_key("gpg.ssh.program"));
    let env = as_env(entries_over(none, &own));
    assert_eq!(value_of(&env, "core.sshCommand").as_deref(), Some("ssh -i C:/keys/id -o IdentitiesOnly=yes"));
    assert_eq!(value_of(&env, "core.editor").as_deref(), Some("code --wait"));
    assert_eq!(value_of(&env, "gpg.program").as_deref(), Some("gpg2"));
    assert_eq!(value_of(&env, "gpg.ssh.program").as_deref(), Some("ssh-keygen"), "neutral where the person sets none");
    assert_eq!(value_of(&env, "core.fsmonitor").as_deref(), Some("false"), "a setting the person does not choose is always neutral");
}

#[test]
fn a_value_in_the_persons_global_configuration_wins_and_the_folders_is_never_used() {
    let folder = Folder::new();
    std::fs::write(
        folder.dir.path().join(".gitconfig"),
        "[core]\n\tsshCommand = ssh -o BatchMode=yes\n\teditor = myeditor --wait\n[gpg]\n\tprogram = mygpg\n[gpg \"ssh\"]\n\tprogram = mysshsign\n",
    )
    .unwrap();
    let own = read_persons(Some(folder.dir.path()));
    assert_eq!(own.get("core.sshcommand").map(String::as_str), Some("ssh -o BatchMode=yes"), "{own:?}");
    assert_eq!(own.get("gpg.ssh.program").map(String::as_str), Some("mysshsign"));
    let given = as_env(entries_over(none, &own));
    let read = |key: &str| String::from_utf8_lossy(&run_git(&folder.repo(), folder.dir.path(), &["config", "--get", key], &given).stdout).trim().to_string();
    assert_eq!(read("core.sshCommand"), "ssh -o BatchMode=yes", "the person's, not the folder's");
    assert_eq!(read("core.editor"), "myeditor --wait");
    assert_eq!(read("gpg.program"), "mygpg");
    assert_eq!(read("gpg.ssh.program"), "mysshsign");
    assert_eq!(read("gpg.x509.program"), "gpgsm", "neutral where the person sets none");
    folder.forget();
    for args in [vec!["status"], vec!["fetch", "origin"], vec!["fetch", "viaext"], vec!["ls-remote", "viassh"]] {
        folder.git(&args, &given);
    }
    assert_eq!(folder.ran(), Vec::<String>::new(), "nothing the folder names runs");
}

#[test]
fn the_brokers_helper_still_answers_and_the_folders_does_not() {
    let folder = Folder::new();
    let broker = HashMap::from([
        ("GIT_CONFIG_COUNT".to_string(), "2".to_string()),
        ("GIT_CONFIG_KEY_0".to_string(), "credential.https://github.com.helper".to_string()),
        ("GIT_CONFIG_VALUE_0".to_string(), String::new()),
        ("GIT_CONFIG_KEY_1".to_string(), "credential.https://github.com.helper".to_string()),
        ("GIT_CONFIG_VALUE_1".to_string(), "!f() { echo username=the-bot; echo password=the-token; }; f".to_string()),
    ]);
    let mut given = broker.clone();
    given.extend(entries_over(|k| broker.get(k).cloned(), &HashMap::new()));
    let all = pairs(&given);
    assert_eq!(all[0], ("credential.helper".to_string(), String::new()), "the reset comes before the broker's list");
    assert_eq!(&all[1..3], &[("credential.https://github.com.helper".to_string(), String::new()), ("credential.https://github.com.helper".to_string(), "!f() { echo username=the-bot; echo password=the-token; }; f".to_string())]);
    folder.forget();
    let ask = |host: &str| {
        let out = run_git_with(&folder.repo(), folder.dir.path(), &["credential", "fill"], &given, &format!("protocol=https\nhost={host}\n\n"));
        String::from_utf8_lossy(&out.stdout).replace("\r\n", "\n")
    };
    assert!(ask("github.com").contains("password=the-token"), "the broker's helper answers for GitHub");
    assert!(!ask("elsewhere.example").contains("password="), "and nothing answers for another host");
    assert_eq!(folder.ran(), Vec::<String>::new(), "the helper the folder names is not run for either");
    // Without the reset, the folder's helper is run for a host the broker's does not cover.
    run_git_with(&folder.repo(), folder.dir.path(), &["credential", "fill"], &broker, "protocol=https\nhost=elsewhere.example\n\n");
    assert_eq!(folder.ran(), vec!["credential"], "what the folder's config runs by itself");
}

#[test]
fn an_agents_own_git_function_is_kept_and_has_the_diff_step() {
    let (data, folder) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    with_data_home(data.path(), || {
        crate::backend::mcp_folder_rule::mark(&folder.path().to_string_lossy());
        let mut env = HashMap::from([(GIT_FUNCTION_NAME.to_string(), agentmux_common::github_broker_shim::GIT_FUNCTION_BODY.to_string())]);
        apply_for_spawn(&mut env, &pane("host", "claude", folder.path()));
        assert_eq!(env[GIT_FUNCTION_NAME], agentmux_common::github_broker_shim::GIT_FUNCTION_BODY);
        assert!(agentmux_common::github_broker_shim::GIT_FUNCTION_BODY.contains("--no-ext-diff"));
        assert_eq!(env[GIT_NO_EXT_DIFF_ENV], "1");
    });
}

/// The bash to run: Git's on Windows (a `bash` on the PATH can be the WSL one), else the PATH's.
fn bash() -> String {
    let git = std::env::var_os("ProgramFiles").map(|p| Path::new(&p).join("Git").join("bin").join("bash.exe"));
    match git {
        Some(path) if cfg!(windows) && path.exists() => path.to_string_lossy().into_owned(),
        _ => "bash".to_string(),
    }
}

/// Runs `script` in bash with `git` defined by `body`, in the folder's repository, with the settings an agent there is
/// given; `flag` says whether the diff step is on. What it printed.
fn through_the_function(folder: &Folder, body: &str, flag: bool, script: &str) -> String {
    let mut env = as_env(entries_over(none, &HashMap::new()));
    if flag {
        env.insert(GIT_NO_EXT_DIFF_ENV.to_string(), "1".to_string());
    }
    let out = run_program(&bash(), &folder.repo(), folder.dir.path(), &["-c", &format!("git{body}\n{script}")], &env, "");
    format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)).replace("\r\n", "\n")
}

#[test]
fn the_function_keeps_the_folders_diff_programs_from_running_and_leaves_git_s_own_options_alone() {
    let folder = Folder::new();
    let repo = folder.repo();
    let script = folder.dir.path().join("mark.sh");
    std::fs::write(repo.join("b.txt"), "x\n").unwrap();
    std::fs::write(repo.join(".gitattributes"), "a.txt diff=drvcmd\nb.txt diff=drvtext\nc.txt filter=fx\nd.txt diff=a=b\ne.txt filter=f=g\n").unwrap();
    std::fs::write(repo.join("c.txt"), "c\n").unwrap();
    std::fs::write(repo.join("d.txt"), "d\n").unwrap();
    std::fs::write(repo.join("e.txt"), "e\n").unwrap();
    std::fs::write(repo.join("w.txt"), "a\nb\nc\nd\ne\n").unwrap();
    folder.git(&["add", "."], &HashMap::new());
    folder.git(&["commit", "-q", "-m", "more"], &HashMap::new());
    std::fs::write(repo.join("b.txt"), "mid\n").unwrap();
    folder.git(&["commit", "-q", "-a", "-m", "again"], &HashMap::new());
    std::fs::write(repo.join("a.txt"), "two\n").unwrap();
    std::fs::write(repo.join("c.txt"), "cc\n").unwrap();
    std::fs::write(repo.join("d.txt"), "dd\n").unwrap();
    std::fs::write(repo.join("e.txt"), "ee\n").unwrap();
    std::fs::write(repo.join("b.txt"), "y\n").unwrap();
    std::fs::write(repo.join("w.txt"), "a\nb\nc \nd\nE\n").unwrap();
    let commands = [
        "git diff",
        "git -C . diff",
        "git -c core.abbrev=9 diff -U0",
        "git --no-pager diff --stat -p",
        "git log -p -1",
        "git show",
        "git format-patch -1 --stdout",
        "git diff HEAD~1 HEAD",
        "git blame b.txt",
        "git reflog -p",
        "git range-diff HEAD~2..HEAD~1 HEAD~1..HEAD",
        "git status",
    ];
    for body in [agentmux_common::github_broker_shim::GIT_FLAGS_FUNCTION_BODY, agentmux_common::github_broker_shim::GIT_FUNCTION_BODY] {
        for (key, name) in [("diff.external", "diffext"), ("diff.drvcmd.command", "drvcmd"), ("diff.drvtext.textconv", "drvtext"), ("filter.fx.clean", "filterclean"), ("filter.fx.process", "filterprocess"), ("diff.a=b.textconv", "eqtext"), ("filter.f=g.clean", "eqclean")] {
            folder.git(&["config", key, &format!("sh {} {name}", slashes(&script))], &HashMap::new());
            folder.forget();
            through_the_function(&folder, body, false, "git diff");
            assert_eq!(folder.ran(), vec![name], "{name}: without the step the folder's program runs");
            folder.forget();
            for command in commands {
                through_the_function(&folder, body, true, command);
            }
            assert_eq!(folder.ran(), Vec::<String>::new(), "{name}: with the step none of {commands:?} runs it");
            folder.git(&["config", "--unset", key], &HashMap::new());
        }
        // Git's own options still work through it.
        let ignoring_whitespace = through_the_function(&folder, body, true, "git diff -w w.txt");
        assert!(ignoring_whitespace.contains("-e") && ignoring_whitespace.contains("+E") && !ignoring_whitespace.contains("+c"), "{ignoring_whitespace}");
        let no_context = through_the_function(&folder, body, true, "git diff -U0 w.txt");
        assert!(!no_context.lines().any(|l| l.starts_with(' ')), "{no_context}");
        let context = through_the_function(&folder, body, true, "git diff -U1 w.txt");
        assert!(context.lines().any(|l| l == " d"), "{context}");
        let stat = through_the_function(&folder, body, true, "git diff --stat");
        assert!(stat.contains("a.txt") && stat.contains("b.txt"), "{stat}");
        let plain = through_the_function(&folder, body, true, "git status --short");
        assert!(plain.contains("a.txt"), "any other command is plain git: {plain}");
    }
}

/// Runs `tool args` on a pseudoterminal in the folder's repository, as git sees a person's terminal (it pages
/// only when its output is one). Ends, or fails, within a minute.
fn on_a_terminal(folder: &Folder, tool: &str, args: &[&str], env: &HashMap<String, String>) -> String {
    let (repo, home) = (folder.repo(), folder.dir.path().to_path_buf());
    let (tool, args, env): (String, Vec<String>, HashMap<String, String>) = (tool.to_string(), args.iter().map(|a| a.to_string()).collect(), env.clone());
    let (done, finished) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let printed = run_on_a_terminal(&repo, &home, &tool, &args, &env);
        let _ = done.send(printed);
    });
    finished.recv_timeout(std::time::Duration::from_secs(60)).expect("git on a terminal ended")
}

fn run_on_a_terminal(repo: &Path, home: &Path, tool: &str, args: &[String], env: &HashMap<String, String>) -> String {
    use portable_pty::{native_pty_system, CommandBuilder, PtySize};
    use std::io::Read;
    let pair = native_pty_system().openpty(PtySize { rows: 30, cols: 100, pixel_width: 0, pixel_height: 0 }).expect("a pty");
    let mut cmd = CommandBuilder::new(tool);
    cmd.args(args);
    cmd.cwd(repo);
    for key in ["GIT_CONFIG_COUNT", "GIT_PROXY_COMMAND", "GIT_SSH_COMMAND", "GIT_EXTERNAL_DIFF", "GIT_PAGER", "PAGER"] {
        cmd.env_remove(key);
    }
    cmd.env("HOME", home);
    cmd.env("USERPROFILE", home);
    cmd.env("GIT_CONFIG_NOSYSTEM", "1");
    cmd.env("GIT_TERMINAL_PROMPT", "0");
    cmd.env("TERM", "xterm");
    for (key, value) in env {
        cmd.env(key, value);
    }
    let mut child = pair.slave.spawn_command(cmd).expect("starts");
    drop(pair.slave);
    let mut reader = pair.master.try_clone_reader().expect("reads");
    let printed = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let sink = printed.clone();
    let mut writer = pair.master.take_writer().expect("writes");
    std::thread::spawn(move || {
        use std::io::Write;
        let mut chunk = [0u8; 4096];
        while let Ok(n) = reader.read(&mut chunk) {
            if n == 0 {
                break;
            }
            // The pseudoconsole asks where the cursor is and waits for the answer before it starts the program.
            if chunk[..n].windows(4).any(|w| w == b"\x1b[6n") {
                let _ = writer.write_all(b"\x1b[1;1R");
                let _ = writer.flush();
            }
            sink.lock().unwrap().extend_from_slice(&chunk[..n]);
        }
    });
    let started = std::time::Instant::now();
    while child.try_wait().ok().flatten().is_none() && started.elapsed() < std::time::Duration::from_secs(30) {
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    let status = child.try_wait().ok().flatten();
    let _ = child.kill();
    std::thread::spawn(move || drop(pair.master));
    std::thread::sleep(std::time::Duration::from_millis(200));
    format!("{status:?} {}", String::from_utf8_lossy(&printed.lock().unwrap()))
}

#[test]
fn a_pager_the_folder_names_for_one_command_does_not_run_on_a_terminal() {
    let folder = Folder::new();
    let given = as_env(entries_over(none, &HashMap::new()));
    folder.forget();
    let mut printed = String::new();
    for command in ["log", "status"] {
        printed += &on_a_terminal(&folder, "git", &[command], &HashMap::new());
    }
    assert_eq!(folder.ran(), vec!["fsmonitor", "pagerlog", "pagerstatus"], "what the folder's config runs by itself on a terminal: {printed:?}");
    folder.forget();
    for command in ["log", "status", "diff", "show", "branch", "tag", "blame"] {
        on_a_terminal(&folder, "git", &[command], &given);
    }
    assert_eq!(folder.ran(), Vec::<String>::new(), "with the settings, no pager the folder names runs");
}

#[test]
fn the_function_keeps_a_pager_the_folder_names_from_running_on_a_terminal() {
    let folder = Folder::new();
    let body = agentmux_common::github_broker_shim::GIT_FLAGS_FUNCTION_BODY;
    let script = format!("git{body}\ngit log; git status");
    let flag = HashMap::from([(GIT_NO_EXT_DIFF_ENV.to_string(), "1".to_string())]);
    folder.forget();
    on_a_terminal(&folder, &bash(), &["-c", &script], &HashMap::new());
    assert_eq!(folder.ran(), vec!["fsmonitor", "pagerlog", "pagerstatus"], "without the step the folder's pagers run");
    folder.forget();
    on_a_terminal(&folder, &bash(), &["-c", &script], &flag);
    let pagers: Vec<String> = folder.ran().into_iter().filter(|name| name.starts_with("pager")).collect();
    assert_eq!(pagers, Vec::<String>::new(), "with it none of the folder's pagers does");
}

#[test]
fn the_function_keeps_a_remotes_upload_and_receive_programs_from_running() {
    let folder = Folder::new();
    let commands = ["git fetch viaup", "git ls-remote viaup", "git pull viaup HEAD", "git push viaup HEAD:refs/heads/x"];
    let remote_commands = ["git remote update viaup", "git remote show viaup", "git remote prune viaup", "git remote -v update", "git remote set-head viaup -a", "git remote add -f other ../remote.git", "git remote add --fe other ../remote.git", "git remote add -ft main other ../remote.git", "git fetch --al", "git fetch --mu viaup", "git pull --al", "git --attr-source HEAD fetch --all", "git fetch --all", "git fetch --multiple viaup", "git pull --all", "git fetch grp"];
    for body in [agentmux_common::github_broker_shim::GIT_FLAGS_FUNCTION_BODY, agentmux_common::github_broker_shim::GIT_FUNCTION_BODY] {
        folder.forget();
        for command in commands {
            through_the_function(&folder, body, false, command);
        }
        assert_eq!(folder.ran(), vec!["receivepack", "uploadpack"], "without the step the remote's programs run");
        folder.forget();
        for command in commands {
            through_the_function(&folder, body, true, command);
        }
        assert_eq!(folder.ran(), Vec::<String>::new(), "with it none of {commands:?} runs them");
    }
    for body in [agentmux_common::github_broker_shim::GIT_FLAGS_FUNCTION_BODY, agentmux_common::github_broker_shim::GIT_FUNCTION_BODY] {
        folder.forget();
        for command in remote_commands {
            let said = through_the_function(&folder, body, true, command);
            assert!(said.contains("is not run while"), "{command}: {said}");
        }
        assert_eq!(folder.ran(), Vec::<String>::new(), "with it none of {remote_commands:?} runs them");
    }
    folder.git(&["config", "fetch.all", "true"], &HashMap::new());
    for body in [agentmux_common::github_broker_shim::GIT_FLAGS_FUNCTION_BODY, agentmux_common::github_broker_shim::GIT_FUNCTION_BODY] {
        folder.forget();
        through_the_function(&folder, body, false, "git fetch");
        assert_eq!(folder.ran(), vec!["uploadpack"], "a fetch of every remote, by the folder's own setting, runs the remote's program");
        folder.forget();
        through_the_function(&folder, body, true, "git fetch");
        assert_eq!(folder.ran(), Vec::<String>::new(), "with the step it fetches the one remote and runs none");
    }
    folder.git(&["config", "--unset", "fetch.all"], &HashMap::new());
    let pushed = through_the_function(&folder, agentmux_common::github_broker_shim::GIT_FLAGS_FUNCTION_BODY, true, "git push viaup HEAD:refs/heads/y && git ls-remote viaup");
    assert!(pushed.contains("refs/heads/y"), "a push still works through it: {pushed}");
}

#[test]
fn the_function_leaves_the_reflog_subcommands_working() {
    let folder = Folder::new();
    for body in [agentmux_common::github_broker_shim::GIT_FLAGS_FUNCTION_BODY, agentmux_common::github_broker_shim::GIT_FUNCTION_BODY] {
        let out = through_the_function(&folder, body, true, "git reflog show -1 && git reflog exists HEAD && git reflog expire --all --dry-run && git reflog list && git reflog HEAD -1 && git reflog -1 && echo reflog-ok");
        assert!(out.contains("reflog-ok"), "{out}");
    }
}

#[test]
fn the_function_keeps_a_merge_driver_the_folder_names_from_running() {
    let folder = Folder::new();
    let repo = folder.repo();
    std::fs::write(repo.join(".gitattributes"), "m.txt merge=mx\n").unwrap();
    std::fs::write(repo.join("m.txt"), "base\n").unwrap();
    folder.git(&["add", "."], &HashMap::new());
    folder.git(&["commit", "-q", "-m", "m0"], &HashMap::new());
    folder.git(&["switch", "-q", "-c", "other"], &HashMap::new());
    std::fs::write(repo.join("m.txt"), "theirs\n").unwrap();
    folder.git(&["commit", "-q", "-a", "-m", "m1"], &HashMap::new());
    folder.git(&["switch", "-q", "-"], &HashMap::new());
    std::fs::write(repo.join("m.txt"), "mine\n").unwrap();
    folder.git(&["commit", "-q", "-a", "-m", "m2"], &HashMap::new());
    let script = folder.dir.path().join("mark.sh");
    folder.git(&["config", "merge.mx.driver", &format!("sh {} mergedriver %O %A %B", slashes(&script))], &HashMap::new());
    for body in [agentmux_common::github_broker_shim::GIT_FLAGS_FUNCTION_BODY, agentmux_common::github_broker_shim::GIT_FUNCTION_BODY] {
        folder.forget();
        through_the_function(&folder, body, false, "git merge other; git merge --abort");
        assert_eq!(folder.ran(), vec!["mergedriver"], "without the step the folder's merge driver runs");
        folder.forget();
        through_the_function(&folder, body, true, "git merge other; git merge --abort");
        assert_eq!(folder.ran(), Vec::<String>::new(), "with it none runs");
    }
}

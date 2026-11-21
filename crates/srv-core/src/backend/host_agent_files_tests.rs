// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::*;
use crate::backend::mcp_config_file::with_data_home;
use serde_json::json;

fn files_under(dir: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(path) = stack.pop() {
        for entry in std::fs::read_dir(&path).into_iter().flatten().flatten() {
            if entry.path().is_dir() {
                stack.push(entry.path());
            } else {
                out.push(entry.path().strip_prefix(dir).unwrap().to_string_lossy().replace('\\', "/"));
            }
        }
    }
    out.sort();
    out
}

fn pane(mode: &str, provider: &str, cwd: &Path) -> crate::backend::obj::MetaMapType {
    [("agentMode", json!(mode)), ("agentProvider", json!(provider)), ("cmd:cwd", json!(cwd.to_string_lossy()))].into_iter().map(|(k, v)| (k.to_string(), v)).collect()
}

const LAUNCH: &[(&str, &str)] = &[
    ("CLAUDE.md", "be kind"),
    (".claude/settings.json", r#"{"hooks":{}}"#),
    (".claude/skills/review/SKILL.md", "---\nname: review\n---\nbody"),
    (".claude/skills/review/notes/reference.md", "a file beside the skill"),
    (".claude/commands/go.md", "go"),
    (".mcp.json", r#"{"mcpServers":{}}"#),
    ("AGENTS.md", "another harness's startup file"),
];

#[test]
fn a_host_launch_there_writes_outside_the_folder_and_nothing_into_it() {
    let (data, folder) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let dir = folder.path().to_string_lossy().into_owned();
    with_data_home(data.path(), || {
        assert_eq!(write(&dir, LAUNCH.iter().copied()), Ok(false), "a folder no container agent has to do with is the caller's to write, as before");
        assert_eq!(dir_of(&dir), None);
        assert!(files_under(data.path()).is_empty() && files_under(folder.path()).is_empty());

        crate::backend::mcp_folder_rule::mark(&dir);
        let own = dir_of(&dir).expect("the folder kept for its host agents");
        assert!(!own.starts_with(folder.path()));
        assert_eq!(write(&dir, LAUNCH.iter().copied()), Ok(true));
        assert!(files_under(folder.path()).is_empty(), "nothing is written into the folder: {:?}", files_under(folder.path()));
        assert_eq!(
            files_under(&own),
            vec![
                "instructions/CLAUDE.md",
                "mcp.json",
                "plugin/.claude-plugin/plugin.json",
                "plugin/commands/go.md",
                "plugin/skills/review/SKILL.md",
                "plugin/skills/review/notes/reference.md",
                "servers-with-no-path.json",
                "settings.json",
            ],
            "each file has its place, and another harness's startup file has none"
        );
        let instructions = std::fs::read_to_string(instructions_path(&own)).unwrap();
        assert!(instructions.starts_with(crate::backend::agent_config::CLAUDE_MD_MANAGED_MARKER) && instructions.ends_with("be kind"), "{instructions}");
        let manifest: Value = serde_json::from_str(&std::fs::read_to_string(plugin_dir(&own).join(".claude-plugin").join("plugin.json")).unwrap()).unwrap();
        assert_eq!(manifest["name"], PLUGIN_NAME);

        // The next launch, with no skill left: the earlier ones are gone, not kept beside the new files.
        let fewer = LAUNCH.iter().copied().filter(|(name, _)| !name.starts_with(".claude/skills") && !name.starts_with(".claude/commands"));
        assert_eq!(write(&dir, fewer), Ok(true));
        assert!(!plugin_dir(&own).exists(), "{:?}", files_under(&own));

        // A refresh writes the instructions again, and only where a launch has put them.
        assert_eq!(refresh_instructions(&dir, "be kinder"), Ok(true));
        assert!(std::fs::read_to_string(instructions_path(&own)).unwrap().ends_with("be kinder"));
        std::fs::remove_dir_all(instructions_dir(&own)).unwrap();
        assert_eq!(refresh_instructions(&dir, "x"), Ok(false));

        // A name that climbs out of its place is refused.
        assert!(write(&dir, [(".claude/skills/../../../elsewhere.md", "x")].into_iter()).is_err());
        assert!(files_under(folder.path()).is_empty());
    });
}

/// Each way a folder comes under the rule gives its host agent a place outside it, and a turn the flags that
/// keep the folder's own files out; a folder no container agent has to do with gets neither.
#[test]
fn every_state_of_the_folder_the_rule_knows_is_given_its_files_from_outside() {
    use crate::backend::fs_ops::container_folders::add_for_test;
    use crate::backend::storage::agents::test_agent_def;
    let data = tempfile::tempdir().unwrap();
    let (bound, root, moved, plain) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    add_for_test(bound.path(), "agentmux-own-files-a", "/workspace");
    let beneath = root.path().join("agents").join("aria");
    std::fs::create_dir_all(&beneath).unwrap();
    add_for_test(&beneath, "agentmux-own-files-b", "/workspace");
    let def = |agent_type: &str, folder: &Path| {
        let mut def = test_agent_def("a-1", "Aria", "claude", agent_type, 1, "");
        def.working_directory = folder.to_string_lossy().into_owned();
        def
    };
    with_data_home(data.path(), || {
        // The switch from a container to the host: the folder was the container's until now.
        crate::backend::mcp_folder_rule::mark_when_moved_to_the_host(&def("container", moved.path()), &def("host", moved.path()));
        let inside = bound.path().join("src");
        for (why, folder) in [
            ("a folder bound into a container", bound.path()),
            ("a folder inside one", inside.as_path()),
            ("a root that holds a container agent's folder", root.path()),
            ("the folder of an agent moved from a container to the host", moved.path()),
        ] {
            let dir = folder.to_string_lossy().into_owned();
            let own = dir_of(&dir).unwrap_or_else(|| panic!("{why}: no place outside"));
            assert!(!own.starts_with(folder), "{why}: {own:?}");
            let flags = crate::backend::agent_runtime::with_runtime_flags(&pane("host", "claude", folder), vec!["-p".to_string()]);
            assert!(flags.windows(2).any(|pair| pair[0] == "--setting-sources" && pair[1] == "user"), "{why}: {flags:?}");
            assert!(flags.iter().any(|flag| flag == "--strict-mcp-config"), "{why}: {flags:?}");
            let mut env = std::collections::HashMap::new();
            apply_for_spawn(&mut env, &pane("host", "claude", folder));
            assert_eq!(env.get(SCHEDULER_OFF).map(String::as_str), Some("1"), "{why}");
        }
        // Where the folder's place can't be told, the rule holds (`mcp_folder_rule::decided`), so the same follows.

        let dir = plain.path().to_string_lossy().into_owned();
        assert_eq!(dir_of(&dir), None, "a folder no container agent has to do with");
        assert_eq!(crate::backend::agent_runtime::with_runtime_flags(&pane("host", "claude", plain.path()), vec!["-p".to_string()]).iter().filter(|flag| flag.contains("setting") || flag.contains("strict") || flag.contains("-dir")).count(), 0);
        let mut env = std::collections::HashMap::new();
        apply_for_spawn(&mut env, &pane("host", "claude", plain.path()));
        assert!(env.is_empty(), "{env:?}");
    });
}

#[test]
fn agentmuxs_hooks_name_the_wrapper_by_its_path_or_are_left_out() {
    let settings = json!({
        "attribution": { "commit": "" },
        "hooks": {
            "PreToolUse": [
                { "matcher": "Bash", "hooks": [{ "type": "command", "command": "my-own-check --strict" }] },
                { "matcher": "^(Bash)$", "hooks": [{ "type": "command", "command": "agentmux-bashwrap hook" }] }
            ],
            "SessionStart": [{ "hooks": [
                { "type": "command", "command": "agentmux-bashwrap sessionstart --part 1" },
                { "type": "command", "command": "agentmux-bashwrap sessionstart --part 2" }
            ] }],
            "Stop": [{ "hooks": [{ "type": "command", "command": "agentmux-bashwrapper-of-my-own" }] }]
        }
    })
    .to_string();
    let wrapper = if cfg!(windows) { Path::new(r"C:\Program Files\AgentMux\tools\bin\agentmux-bashwrap.exe") } else { Path::new("/opt/agent mux/tools/bin/agentmux-bashwrap") };
    let shown = wrapper.to_string_lossy().replace('\\', "/");

    let by_path: Value = serde_json::from_str(&hooks_by_path(&settings, Some(wrapper))).unwrap();
    assert_eq!(by_path["hooks"]["PreToolUse"][1]["hooks"][0]["command"], json!(format!("\"{shown}\" hook")), "one quoted word, a folder with a space included");
    assert_eq!(by_path["hooks"]["SessionStart"][0]["hooks"][1]["command"], json!(format!("\"{shown}\" sessionstart --part 2")));
    assert_eq!(by_path["hooks"]["PreToolUse"][0]["hooks"][0]["command"], "my-own-check --strict", "the agent's own hook is as the person wrote it");
    assert_eq!(by_path["hooks"]["Stop"][0]["hooks"][0]["command"], "agentmux-bashwrapper-of-my-own", "a name that only starts like the wrapper's is not the wrapper");
    assert_eq!(by_path["attribution"]["commit"], "", "the rest of the settings stays");
    assert!(!by_path.to_string().contains("\"agentmux-bashwrap "), "no hook names the wrapper barely: {by_path}");

    let without: Value = serde_json::from_str(&hooks_by_path(&settings, None)).unwrap();
    assert_eq!(without["hooks"]["PreToolUse"].as_array().unwrap().len(), 1, "with no wrapper outside the folder: no hook, not a name");
    assert_eq!(without["hooks"]["PreToolUse"][0]["hooks"][0]["command"], "my-own-check --strict");
    assert_eq!(without["hooks"]["SessionStart"], json!([]));
    assert!(!without.to_string().contains("agentmux-bashwrap "), "{without}");

    assert_eq!(hooks_by_path("not json", Some(wrapper)), "not json");
    assert!(!plain_in_quotes(Path::new("/opt/a\"b/agentmux-bashwrap")) && !plain_in_quotes(Path::new("/opt/$HOME/agentmux-bashwrap")));

    // As a launch calls it: whatever ships on this computer, no hook is left that names the wrapper barely.
    let (data, folder) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    with_data_home(data.path(), || {
        let dir = folder.path().to_string_lossy().into_owned();
        crate::backend::mcp_folder_rule::mark(&dir);
        let text = settings_with_hooks_by_path(&settings, &dir);
        assert!(!text.contains("\"agentmux-bashwrap ") && text.contains("my-own-check --strict"), "{text}");
    });
}

#[test]
fn what_is_left_out_is_counted_by_kind_and_agentmuxs_own_files_are_not_the_folders() {
    let folder = tempfile::tempdir().unwrap();
    let put = |rel: &str, text: &str| {
        let path = folder.path().join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    };
    let dir = folder.path().to_string_lossy().into_owned();
    assert!(left_out(&dir).is_empty(), "an empty folder");

    // What AgentMux itself wrote there at an earlier launch.
    put("CLAUDE.md", &format!("{}\n\nmine", crate::backend::agent_config::CLAUDE_MD_MANAGED_MARKER));
    put(".claude/settings.json", r#"{"hooks":{"PreToolUse":[{"hooks":[{"type":"command","command":"agentmux-bashwrap hook"}]}]}}"#);
    put(".claude/skills/given/SKILL.md", "x");
    put(".claude/commands/given.md", "x");
    put(crate::backend::agent_config::MANAGED_SKILL_FILES_MANIFEST, r#"[".claude/skills/given/SKILL.md",".claude/commands/given.md"]"#);
    assert!(left_out(&dir).is_empty(), "AgentMux's own startup file, settings and skills are given from outside: {:?}", left_out(&dir));

    // The folder's own, one kind at a time.
    put(".claude/settings.local.json", "{}");
    assert_eq!(left_out(&dir), LeftOut { settings: 1, ..LeftOut::default() });
    put(".claude/settings.json", r#"{"hooks":{"Stop":[{"hooks":[{"type":"command","command":"anything"}]}]}}"#);
    assert_eq!(left_out(&dir).settings, 2);
    put("CLAUDE.md", "the folder's own");
    put(".claude/CLAUDE.md", "x");
    put("CLAUDE.local.md", "x");
    put(".claude/rules/style.md", "x");
    put(".claude/rules/deep/er/more.md", "x");
    put(".claude/rules/not-a-rule.txt", "x");
    assert_eq!(left_out(&dir).instructions, 5);
    put(".claude/skills/theirs/SKILL.md", "x");
    put(".claude/skills/theirs/helper.py", "x");
    assert_eq!(left_out(&dir).skills, 1);
    put(".claude/commands/theirs.md", "x");
    put(".claude/commands/group/other.md", "x");
    assert_eq!(left_out(&dir).commands, 2);
    put(".claude/agents/helper.md", "x");
    assert_eq!(left_out(&dir).agents, 1);
    put(".claude/output-styles/terse.md", "x");
    put(".claude/workflows/ship.js", "x");
    put(".claude/scheduled_tasks.json", "{}");
    assert_eq!(left_out(&dir), LeftOut { settings: 2, instructions: 5, skills: 1, commands: 2, agents: 1, other: 3 });

    // A folder with very many files is counted to a limit, not walked to the end.
    for n in 0..(MOST_COUNTED + 50) {
        put(&format!(".claude/agents/many-{n}.md"), "x");
    }
    assert_eq!(left_out(&dir).agents, MOST_COUNTED);
}

#[test]
fn only_a_host_claude_agent_in_such_a_folder_has_its_scheduler_off() {
    let (data, folder, other) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    with_data_home(data.path(), || {
        crate::backend::mcp_folder_rule::mark(&folder.path().to_string_lossy());
        let env_of = |meta: &crate::backend::obj::MetaMapType| {
            let mut env = std::collections::HashMap::from([(SCHEDULER_OFF.to_string(), "0".to_string()), ("KEPT".to_string(), "yes".to_string())]);
            apply_for_spawn(&mut env, meta);
            env
        };
        let there = env_of(&pane("host", "claude", folder.path()));
        assert_eq!(there.get(SCHEDULER_OFF).map(String::as_str), Some("1"), "over the agent's own value");
        assert_eq!(there.get(READ_ADDED_INSTRUCTIONS).map(String::as_str), Some("1"));
        assert_eq!(there.get("KEPT").map(String::as_str), Some("yes"));
        assert_eq!(env_of(&pane("host", "claude-code", folder.path())).get(SCHEDULER_OFF).map(String::as_str), Some("1"), "by any name the harness goes by");

        for (why, meta) in [
            ("a folder no container agent has to do with", pane("host", "claude", other.path())),
            ("the container agent itself", pane("container", "claude", folder.path())),
            ("another harness, which is not started there at all", pane("host", "codex", folder.path())),
        ] {
            let env = env_of(&meta);
            assert_eq!(env.get(SCHEDULER_OFF).map(String::as_str), Some("0"), "{why}: left as it was");
            assert!(!env.contains_key(READ_ADDED_INSTRUCTIONS), "{why}");
        }
    });
}

/// The flags and variables of this module keep the folder's files out of the CLI because the CLI was seen to
/// behave so, kind by kind, in a live run. That run was made with the version named here. When the pinned
/// version moves, run the check again against the new one, then change the constant.
#[test]
fn the_per_kind_check_was_run_with_the_pinned_claude_code() {
    let pinned = crate::backend::providers::get_provider("claude").expect("the claude provider").pinned_version;
    assert_eq!(
        CHECKED_WITH_CLAUDE_CODE, pinned,
        "the pinned Claude Code moved to {pinned}: run the per-kind check of what the CLI loads from its working folder against it before changing CHECKED_WITH_CLAUDE_CODE"
    );
}

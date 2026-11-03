use super::*;

fn env(pairs: &[(&str, &str)]) -> HashMap<String, String> {
    pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
}

/// A fixed answer for the one command the tests resolve, so they do not depend on the machine's PATH.
fn abs(cmd: &str) -> Option<String> {
    (cmd == "uvx").then(|| "/bin/uvx".to_string())
}

fn pairs(made: &CodexMcp) -> Vec<String> {
    made.overrides
        .chunks(2)
        .map(|c| {
            assert_eq!(c[0], "-c");
            c[1].clone()
        })
        .collect()
}

fn has_env(made: &CodexMcp, name: &str, value: &str) -> bool {
    made.env.iter().any(|(k, v)| k == name && v == value)
}

const MCP: &str = r#"{"mcpServers":{
    "agentmux":{"type":"stdio","command":"agentmux-mcp","args":[],"env":{"AGENTMUX_AGENT_ID":"aria","N":3}},
    "git":{"command":"uvx","args":["mcp-server-git","--repo","C:\\work"],"env":{"TOKEN":"${TOKEN}","MODE":"fast"}},
    "web":{"type":"http","url":"https://example.com/mcp","headers":{"X-A":"b","X-Key":"${KEY_VAR}","Authorization":"Bearer ${GH_TOKEN}"}},
    "old":{"type":"sse","url":"https://example.com/sse"},
    "off":{"command":"x","disabled":true}
}}"#;

#[test]
fn a_server_becomes_dotted_overrides() {
    let made = codex_mcp_overrides(MCP, &env(&[]), &abs);
    let all = pairs(&made);
    assert!(all.contains(&r#"mcp_servers.git.command="/bin/uvx""#.to_string()), "{all:?}");
    assert!(all.contains(&r#"mcp_servers.git.args=["mcp-server-git","--repo","C:\\work"]"#.to_string()), "{all:?}");
    assert!(all.contains(&r#"mcp_servers.git.env_vars=["MODE","TOKEN"]"#.to_string()), "{all:?}");
    assert!(all.contains(&r#"mcp_servers.web.url="https://example.com/mcp""#.to_string()), "{all:?}");
}

/// A literal value is as likely a key as a `${VAR}` one: it travels in the
/// environment Codex is launched with, never in a `-c` override.
#[test]
fn an_env_or_header_value_never_reaches_the_command_line() {
    let made = codex_mcp_overrides(MCP, &env(&[("TOKEN", "s3cret"), ("KEY_VAR", "k3y"), ("GH_TOKEN", "g3t")]), &abs);
    let all = pairs(&made).join("\n");
    for value in ["s3cret", "k3y", "g3t", "fast", "\"b\""] {
        assert!(!all.contains(value), "{value} reached the command line: {all}");
    }
    assert!(!all.contains(".env=") && !all.contains(".http_headers="), "{all}");
    // Forwarded by name; the literal ones are put in the launch environment.
    assert!(all.contains(r#"mcp_servers.web.env_http_headers={X-A="AGENTMUX_MCP_S2_H1",X-Key="KEY_VAR"}"#), "{all}");
    assert!(all.contains(r#"mcp_servers.web.bearer_token_env_var="GH_TOKEN""#), "{all}");
    assert!(has_env(&made, "MODE", "fast") && has_env(&made, "AGENTMUX_MCP_S2_H1", "b"), "{:?}", made.env);
    assert!(!made.env.iter().any(|(k, _)| k == "TOKEN" || k == "KEY_VAR"), "a ${{VAR}} is read from the launch environment: {:?}", made.env);
}

/// The agent's signing keys are in the `agentmux` server's `env` in `.mcp.json`.
#[test]
fn the_signing_keys_in_the_agentmux_entry_stay_off_the_command_line() {
    let raw = r#"{"mcpServers":{"agentmux":{"command":"agentmux-mcp","env":{"AGENTMUX_AGENT_ID":"aria","AGENTMUX_JEKT_KEY":"PRIVATE-JEKT","AGENTMUX_LAN_KEY":"PRIVATE-LAN"}}}}"#;
    let made = codex_mcp_overrides(raw, &env(&[]), &abs);
    let all = pairs(&made).join("\n");
    assert!(!all.contains("PRIVATE"), "{all}");
    assert!(all.contains(r#"mcp_servers.agentmux.env_vars=["AGENTMUX_AGENT_ID","AGENTMUX_JEKT_KEY","AGENTMUX_LAN_KEY"]"#), "{all}");
    assert!(has_env(&made, "AGENTMUX_JEKT_KEY", "PRIVATE-JEKT") && has_env(&made, "AGENTMUX_LAN_KEY", "PRIVATE-LAN"), "{:?}", made.env);
}

#[test]
fn agentmux_forwards_the_agents_agentmux_variables() {
    let made = codex_mcp_overrides(MCP, &env(&[("AGENTMUX_AUTH_KEY", "k"), ("AGENTMUX_BLOCKID", "b"), ("AGENTMUX_AGENT_ID", "Aria"), ("OTHER", "x")]), &abs);
    let all = pairs(&made);
    assert!(
        all.contains(&r#"mcp_servers.agentmux.env_vars=["AGENTMUX_AGENT_ID","N","AGENTMUX_AUTH_KEY","AGENTMUX_BLOCKID"]"#.to_string()),
        "{all:?}"
    );
    // The agent's own AGENTMUX_AGENT_ID wins over the copy in `.mcp.json`; the process's is not changed.
    assert!(!made.env.iter().any(|(k, _)| k == "AGENTMUX_AGENT_ID"), "{:?}", made.env);
    assert!(has_env(&made, "N", "3"));
}

#[test]
fn servers_codex_cannot_take_are_left_out_and_named() {
    let made = codex_mcp_overrides(MCP, &env(&[]), &abs);
    assert!(made.skipped.iter().any(|(n, _)| n == "old"));
    assert!(!made.skipped.iter().any(|(n, _)| n == "off"), "a disabled server is simply absent");
    assert!(!pairs(&made).iter().any(|l| l.contains(".off.") || l.contains(".old.")));
}

#[test]
fn a_secret_mixed_into_text_leaves_the_server_out() {
    let raw = r#"{"mcpServers":{
        "a":{"command":"x","env":{"URL":"https://u:${PW}@h"}},
        "b":{"type":"http","url":"https://h","headers":{"X":"pre-${V}"}},
        "c":{"command":"y"}}}"#;
    let made = codex_mcp_overrides(raw, &env(&[("PW", "hunter2"), ("V", "vv")]), &abs);
    let names: Vec<&str> = made.skipped.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(names, vec!["a", "b"]);
    let all = pairs(&made).join("\n");
    assert!(all.contains("mcp_servers.c.command") && !all.contains("hunter2") && !all.contains("mcp_servers.a."), "{all}");
}

#[test]
fn a_name_codex_refuses_and_odd_values_cannot_inject_keys() {
    let raw = r#"{"mcpServers":{"has space":{"command":"x"},"ok":{"command":"a\"b","args":["x\" ]\nfoo=1"],"env":{"K":"v\"}"}}}}"#;
    let made = codex_mcp_overrides(raw, &env(&[]), &abs);
    assert_eq!(made.skipped.len(), 1);
    for line in pairs(&made) {
        assert!(line.starts_with("mcp_servers.ok."), "{line}");
        assert!(!line.contains('\n'), "{line:?}");
    }
    assert!(has_env(&made, "K", "v\"}"), "{:?}", made.env);
}

/// A name Codex can't forward by name leaves the server out.
#[test]
fn an_env_name_that_cannot_be_forwarded_leaves_the_server_out() {
    let raw = r#"{"mcpServers":{"a":{"command":"x","env":{"K ey":"v"}},"b":{"command":"y","env":{"1ST":"v"}}}}"#;
    let made = codex_mcp_overrides(raw, &env(&[]), &abs);
    assert_eq!(made.skipped.len(), 2);
    assert!(made.overrides.is_empty() && made.env.is_empty());
}

/// The same name set to two values, or to one the agent's launch already has,
/// can't both be right in one process: the later server is left out.
#[test]
fn a_name_set_to_two_values_leaves_the_later_server_out() {
    let raw = r#"{"mcpServers":{
        "a":{"command":"x","env":{"MODE":"fast"}},
        "b":{"command":"y","env":{"MODE":"slow"}},
        "c":{"command":"z","env":{"MODE":"fast"}},
        "d":{"command":"w","env":{"HOME":"/elsewhere"}}}}"#;
    let made = codex_mcp_overrides(raw, &env(&[("HOME", "/home/me")]), &abs);
    let names: Vec<&str> = made.skipped.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(names, vec!["b", "d"]);
    let all = pairs(&made).join("\n");
    assert!(all.contains("mcp_servers.a.") && all.contains("mcp_servers.c.") && !all.contains("mcp_servers.b."), "{all}");
    assert!(has_env(&made, "MODE", "fast") && !has_env(&made, "MODE", "slow"));
}

#[test]
fn overrides_go_right_after_exec() {
    let argv: Vec<String> = ["exec", "--json", "-"].iter().map(|s| s.to_string()).collect();
    let o: Vec<String> = ["-c", "a=1"].iter().map(|s| s.to_string()).collect();
    assert_eq!(with_overrides(&argv, &o), vec!["exec", "-c", "a=1", "--json", "-"]);
    assert_eq!(with_overrides(&argv, &[]), argv);
    let no_exec: Vec<String> = vec!["--json".into()];
    assert_eq!(with_overrides(&no_exec, &o), no_exec);
}

#[test]
fn not_json_or_no_servers_gives_nothing() {
    assert_eq!(codex_mcp_overrides("nope", &env(&[]), &abs), CodexMcp::default());
    assert_eq!(codex_mcp_overrides("{}", &env(&[]), &abs), CodexMcp::default());
}

/// A variable in a command, an argument or a url would be written out as its
/// value on the command line, so the server is left out instead.
#[test]
fn a_variable_in_a_command_argument_or_url_leaves_the_server_out() {
    let raw = r#"{"mcpServers":{
        "arg":{"command":"x","args":["--token","${GH_TOKEN}"]},
        "cmd":{"command":"${HOME}/bin/x"},
        "url":{"type":"http","url":"https://h/mcp?key=${KEY}"},
        "ok":{"command":"y","args":["--plain"]}}}"#;
    let made = codex_mcp_overrides(raw, &env(&[("GH_TOKEN", "g3t"), ("KEY", "k3y"), ("HOME", "/home/me")]), &abs);
    let names: Vec<&str> = made.skipped.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(names, vec!["arg", "cmd", "url"]);
    let all = pairs(&made).join("\n");
    assert!(all.contains("mcp_servers.ok.args=[\"--plain\"]"), "{all}");
    for leaked in ["g3t", "k3y", "/home/me", "mcp_servers.arg.", "mcp_servers.cmd.", "mcp_servers.url."] {
        assert!(!all.contains(leaked), "{leaked} reached the command line: {all}");
    }
}

/// srv's endpoint is in its own environment, not in the turn's: `agentmux-mcp`
/// still needs it, so it is forwarded by name with the rest.
#[test]
fn the_endpoint_srv_inherits_is_forwarded_to_agentmux() {
    let inherited = [("AGENTMUX_LOCAL_URL", "http://127.0.0.1:1"), ("AGENTMUX_CHANNEL", "dev"), ("PATH", "/bin")].map(|(k, v)| (k.to_string(), v.to_string()));
    let launch = launch_env(inherited.into_iter(), &env(&[("AGENTMUX_BLOCKID", "b"), ("AGENTMUX_LOCAL_URL", "http://own")]));
    assert_eq!(launch.get("AGENTMUX_LOCAL_URL").map(String::as_str), Some("http://own"), "the turn's own value wins");
    assert!(!launch.contains_key("AGENTMUX_CHANNEL") && launch.contains_key("PATH"), "what Codex has: {launch:?}");
    let raw = r#"{"mcpServers":{"agentmux":{"command":"agentmux-mcp","env":{"AGENTMUX_AGENT_ID":"aria"}}}}"#;
    let inherited_only = launch_env([("AGENTMUX_LOCAL_URL".to_string(), "u".to_string())].into_iter(), &env(&[]));
    let all = pairs(&codex_mcp_overrides(raw, &inherited_only, &abs)).join("
");
    assert!(all.contains(r#"mcp_servers.agentmux.env_vars=["AGENTMUX_AGENT_ID","AGENTMUX_LOCAL_URL"]"#), "{all}");
}

/// A server's literal value must not replace a variable Codex has from srv's
/// own environment (its API key, its proxy, its home), which the turn's small
/// overlay does not list.
#[test]
fn a_literal_env_value_never_replaces_a_variable_srv_passes_on() {
    let raw = r#"{"mcpServers":{
        "a":{"command":"x","env":{"OPENAI_API_KEY":"other","NODE_OPTIONS":"--max-old-space-size=1"}},
        "b":{"command":"y","env":{"HTTPS_PROXY":"http://p"}}}}"#;
    let launch = launch_env([("OPENAI_API_KEY", "mine"), ("HTTPS_PROXY", "http://p")].map(|(k, v)| (k.to_string(), v.to_string())).into_iter(), &env(&[]));
    let made = codex_mcp_overrides(raw, &launch, &abs);
    let names: Vec<&str> = made.skipped.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(names, vec!["a"], "the same value is no change; another is");
    assert!(has_env(&made, "HTTPS_PROXY", "http://p") && !made.env.iter().any(|(k, _)| k == "OPENAI_API_KEY"), "{:?}", made.env);
}

/// `"GITHUB_PERSONAL_ACCESS_TOKEN": "${GITHUB_TOKEN}"`: the server's name for a
/// variable differs from the agent's. Staged under the server's name, so the
/// value is forwarded by name and still not on the command line.
#[test]
fn a_whole_value_placeholder_with_another_name_is_staged_under_the_servers_name() {
    let raw = r#"{"mcpServers":{
        "gh":{"command":"x","env":{"GITHUB_PERSONAL_ACCESS_TOKEN":"${GITHUB_TOKEN}","OPTIONAL":"${NOT_SET}"}},
        "other":{"command":"y","env":{"GITHUB_PERSONAL_ACCESS_TOKEN":"${DIFFERENT_TOKEN}"}}}}"#;
    let launch = env(&[("GITHUB_TOKEN", "ghp_secret"), ("DIFFERENT_TOKEN", "ghp_other")]);
    let made = codex_mcp_overrides(raw, &launch, &abs);
    let all = pairs(&made).join("
");
    assert!(!all.contains("ghp_"), "{all}");
    assert!(all.contains(r#"mcp_servers.gh.env_vars=["GITHUB_PERSONAL_ACCESS_TOKEN"]"#), "an unset source gives no entry: {all}");
    assert!(has_env(&made, "GITHUB_PERSONAL_ACCESS_TOKEN", "ghp_secret"), "{:?}", made.env);
    // Two servers wanting the one name from two sources can't both have it.
    let names: Vec<&str> = made.skipped.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(names, vec!["other"], "{:?}", made.skipped);
}

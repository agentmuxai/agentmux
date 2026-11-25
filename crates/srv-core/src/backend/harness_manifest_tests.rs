// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::*;
use serde_json::{json, Value};

fn sha() -> String {
    "ab".repeat(32)
}

fn good() -> Value {
    json!({
        "manifestVersion": 1,
        "id": "custom-demo",
        "name": "Demo",
        "protocol": "acp",
        "command": "demo-acp",
        "args": ["--stdio"],
        "install": { "kind": "path" }
    })
}

fn parse(v: &Value) -> Result<HarnessManifest, ManifestError> {
    HarnessManifest::parse(&v.to_string())
}

/// The manifest with one field set (a dotted path into the object).
fn with(field: &str, value: Value) -> Value {
    let mut v = good();
    v[field] = value;
    v
}

fn refused_field(v: &Value) -> String {
    parse(v).expect_err("the manifest must be refused").field
}

#[test]
fn a_minimal_manifest_is_accepted() {
    let m = parse(&good()).expect("accepted");
    assert_eq!(m.id, "custom-demo");
    assert_eq!(m.auth, Auth::None);
    assert_eq!(m.install, Install::Path);
}

#[test]
fn a_full_manifest_is_accepted() {
    let v = json!({
        "manifestVersion": 1,
        "id": "custom-full-1",
        "name": "Full one",
        "icon": "logo.svg",
        "protocol": "acp",
        "command": "full-acp",
        "args": ["acp", "--quiet"],
        "env": { "FULL_QUIET": "1" },
        "auth": { "kind": "key", "envVar": "FULL_API_KEY" },
        "startupInstructions": "AGENTS.md",
        "vendors": ["openai", "anthropic"],
        "baseUrlEnvVar": "FULL_BASE_URL",
        "install": { "kind": "npm", "package": "@scope/full-acp", "version": "1.2.3-beta.1" }
    });
    assert!(parse(&v).is_ok());
    let dl = json!({
        "manifestVersion": 1, "id": "custom-dl", "name": "Dl", "protocol": "acp", "command": "dl",
        "auth": { "kind": "config-dir", "envVar": "DL_HOME" },
        "install": { "kind": "download", "urls": { "linux-x86_64": { "url": "https://example.com/dl", "sha256": sha() } } }
    });
    assert!(parse(&dl).is_ok());
}

#[test]
fn an_unknown_field_or_bad_json_is_refused() {
    let mut v = good();
    v["shell"] = json!("rm -rf /");
    assert_eq!(refused_field(&v), "(file)");
    assert_eq!(HarnessManifest::parse("{ not json").unwrap_err().field, "(file)");
}

#[test]
fn the_version_and_protocol_are_pinned() {
    assert_eq!(refused_field(&with("manifestVersion", json!(2))), "manifestVersion");
    assert_eq!(refused_field(&with("protocol", json!("msp"))), "protocol");
}

#[test]
fn the_id_must_carry_the_prefix_and_a_clean_name() {
    for bad_id in ["demo", "claude", "custom-", "custom-Demo", "custom-a_b", "custom--x", "custom-x-", "custom-a b"] {
        assert_eq!(refused_field(&with("id", json!(bad_id))), "id", "{bad_id}");
    }
    let long = format!("custom-{}", "a".repeat(60));
    assert_eq!(refused_field(&with("id", json!(long))), "id");
}

#[test]
fn the_name_is_plain_text_of_bounded_length() {
    for bad_name in [json!(""), json!("   "), json!("a\nb"), json!("x".repeat(65))] {
        assert_eq!(refused_field(&with("name", bad_name)), "name");
    }
}

#[test]
fn an_icon_is_a_glyph_or_a_png_or_svg_in_the_folder() {
    for ok in ["bot", "logo.png", "Logo.SVG"] {
        assert!(parse(&with("icon", json!(ok))).is_ok(), "{ok}");
    }
    for bad_icon in ["../logo.png", "a/b.png", "logo.exe", "C:logo.png", "..", "", "Bot"] {
        assert_eq!(refused_field(&with("icon", json!(bad_icon))), "icon", "{bad_icon}");
    }
}

#[test]
fn the_command_is_a_program_never_a_command_string() {
    let absolute = if cfg!(windows) { "C:\\tools\\demo.exe" } else { "/opt/demo/demo" };
    assert!(parse(&with("command", json!(absolute))).is_ok());
    for bad_cmd in ["", "demo --stdio", "demo; rm x", "./demo", "..\\demo", "a/b", "demo\n", "$(demo)"] {
        assert_eq!(refused_field(&with("command", json!(bad_cmd))), "command", "{bad_cmd:?}");
    }
}

#[test]
fn arguments_are_an_array_with_limits() {
    assert_eq!(refused_field(&with("args", json!(["a\u{0}b"]))), "args[0]");
    assert_eq!(refused_field(&with("args", json!(vec!["x"; 65]))), "args");
    assert_eq!(refused_field(&with("args", json!("--stdio"))), "(file)");
}

#[test]
fn env_names_are_not_agentmuxs_or_path() {
    assert!(parse(&with("env", json!({ "DEMO_FLAG": "1" }))).is_ok());
    for name in ["AGENTMUX_TOKEN", "agentmux_x", "PATH", "Path", "1BAD", "A-B", ""] {
        let field = refused_field(&with("env", json!({ name: "x" })));
        assert_eq!(field, format!("env.{name}"));
    }
    assert_eq!(refused_field(&with("env", json!({ "OK": "a\u{0}b" }))), "env.OK");
}

#[test]
fn a_sign_in_variable_is_never_fixed_in_env() {
    let mut v = with("auth", json!({ "kind": "key", "envVar": "DEMO_KEY" }));
    assert!(parse(&v).is_ok());
    v["env"] = json!({ "DEMO_KEY": "sk-secret" });
    assert_eq!(refused_field(&v), "auth.envVar");
    let mut b = with("auth", json!({ "kind": "key", "envVar": "AGENTMUX_X" }));
    assert_eq!(refused_field(&b), "auth.envVar");
    b["auth"] = json!({ "kind": "oauth", "envVar": "X" });
    assert_eq!(refused_field(&b), "(file)");
}

#[test]
fn the_startup_file_is_a_plain_file_name() {
    assert!(parse(&with("startupInstructions", json!("AGENTS.md"))).is_ok());
    for bad_file in ["", "a/b.md", "..\\x.md", "../x", "C:x.md", "x\u{0}.md", ".."] {
        assert_eq!(refused_field(&with("startupInstructions", json!(bad_file))), "startupInstructions", "{bad_file:?}");
    }
}

#[test]
fn vendors_and_the_base_url_variable_are_checked() {
    assert_eq!(refused_field(&with("vendors", json!(["OpenAI"]))), "vendors[0]");
    assert_eq!(refused_field(&with("vendors", json!(vec!["a"; 9]))), "vendors");
    assert_eq!(refused_field(&with("baseUrlEnvVar", json!("PATH"))), "baseUrlEnvVar");
    let mut v = with("baseUrlEnvVar", json!("DEMO_URL"));
    v["env"] = json!({ "DEMO_URL": "https://x.example" });
    assert_eq!(refused_field(&v), "baseUrlEnvVar");
}

#[test]
fn an_npm_install_needs_an_exact_version_and_a_bare_command() {
    let npm = |package: &str, version: &str| json!({ "kind": "npm", "package": package, "version": version });
    assert!(parse(&with("install", npm("demo-acp", "0.1.0"))).is_ok());
    for version in ["latest", "^1.2.3", "1.2", "1.2.x", "", ">=1.0.0", "1.2.3 "] {
        assert_eq!(refused_field(&with("install", npm("demo-acp", version))), "install.version", "{version}");
    }
    for package in ["", "Demo", "../x", "a/b", "@s/a/b", "git+https://x/y", "-x"] {
        assert_eq!(refused_field(&with("install", npm(package, "1.0.0"))), "install.package", "{package}");
    }
    let absolute = if cfg!(windows) { "C:\\x\\demo.exe" } else { "/x/demo" };
    let mut v = with("install", npm("demo-acp", "1.0.0"));
    v["command"] = json!(absolute);
    assert_eq!(refused_field(&v), "command");
}

#[test]
fn a_download_needs_https_a_sha256_and_a_known_platform() {
    let dl = |platform: &str, url: &str, sha: &str| {
        json!({ "kind": "download", "urls": { platform: { "url": url, "sha256": sha } } })
    };
    assert!(parse(&with("install", dl("macos-aarch64", "https://example.com/a.tgz", &sha()))).is_ok());
    assert_eq!(refused_field(&with("install", dl("beos-x86", "https://example.com/a", &sha()))), "install.urls.beos-x86");
    assert_eq!(
        refused_field(&with("install", dl("linux-x86_64", "http://example.com/a", &sha()))),
        "install.urls.linux-x86_64.url"
    );
    for sha_bad in ["", "AB".repeat(32).as_str(), &"a".repeat(63)] {
        assert_eq!(
            refused_field(&with("install", dl("linux-x86_64", "https://example.com/a", sha_bad))),
            "install.urls.linux-x86_64.sha256"
        );
    }
    assert_eq!(refused_field(&with("install", json!({ "kind": "download", "urls": {} }))), "install.urls");
}

#[test]
fn the_error_names_the_file_and_the_field() {
    let e = parse(&with("protocol", json!("x"))).unwrap_err();
    assert!(e.to_string().starts_with("harness.json: protocol: "), "{e}");
}

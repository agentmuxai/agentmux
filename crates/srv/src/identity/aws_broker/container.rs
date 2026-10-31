// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What a container agent is given so its AWS SDK and CLI find the broker (spec §4): a config file with one
//! `credential_process` profile per grant, mounted read-only, and up to four environment variables. The file is the
//! same for every agent (who may have which grant is srv's decision on each request, not the file's), so there is one
//! folder, not one per agent: no agent name is ever a folder name.
//!
//! Nothing here is a security boundary (an agent can unset the variables or write its own config): srv decides on every
//! request. The file is a convenience that makes `aws`, `cdk` and the SDKs ask the right place, and it holds names and
//! one command, never a secret.

use std::path::{Path, PathBuf};

use super::policy::{self, Policy};

/// Where the config folder is mounted, read-only, in the container.
pub const CONTAINER_AWS_DIR: &str = "/opt/agentmux/aws";
const CONFIG_FILE: &str = "config";

/// Write the AWS config under `root` (`container-aws/config`) if it differs from what is there, and return the folder.
/// A new file replaces the old one in one step, so a container reading the mount never sees half of it.
pub fn render_into(root: &Path, policy: &Policy) -> std::io::Result<PathBuf> {
    let dir = root.join("container-aws");
    std::fs::create_dir_all(&dir)?;
    let file = dir.join(CONFIG_FILE);
    let text = policy.render_config();
    if std::fs::read_to_string(&file).ok().as_deref() != Some(text.as_str()) {
        let temp = dir.join(format!(".{CONFIG_FILE}.{}", std::process::id()));
        std::fs::write(&temp, &text)?;
        std::fs::rename(&temp, &file)?;
    }
    Ok(dir)
}

/// The variables a container's exec gets when `policy` shares grants. `AWS_PROFILE` is the policy's default grant, so
/// a bare `aws s3 ls` uses it; without one, a profile is named per command.
pub fn env_for(policy: &Policy) -> Vec<(String, String)> {
    let mut env = vec![
        ("AWS_CONFIG_FILE".to_string(), format!("{CONTAINER_AWS_DIR}/{CONFIG_FILE}")),
        ("AWS_SHARED_CREDENTIALS_FILE".to_string(), "/dev/null".to_string()),
        ("AWS_EC2_METADATA_DISABLED".to_string(), "true".to_string()),
    ];
    if let Some(default) = &policy.default_grant {
        env.push(("AWS_PROFILE".to_string(), default.clone()));
    }
    env
}

/// The policy now in force, if it shares anything.
fn shared() -> Option<std::sync::Arc<Policy>> {
    policy::current().ok().flatten().filter(|p| !p.grants.is_empty())
}

/// The host folder to mount, with the config freshly written, or `None` when this host shares no AWS (or the folder
/// can't be written: the container then has no AWS, as before).
pub fn config_dir() -> Option<String> {
    // The machine running the tests has its own answer; the tests give theirs.
    if cfg!(test) {
        return None;
    }
    let policy = shared()?;
    let root = crate::srv_info::agentmux_root()?;
    match render_into(&root, &policy) {
        Ok(dir) => Some(dir.to_string_lossy().into_owned()),
        Err(e) => {
            tracing::warn!(target: "identity", error = %e, "identity.aws: the container's AWS config couldn't be written");
            None
        }
    }
}

/// The variables to add to a container exec's environment: none when nothing is shared, and none when the config
/// folder can't be written, so they never point the SDK at a file the container doesn't have.
pub fn exec_env() -> Vec<(String, String)> {
    if cfg!(test) {
        return Vec::new();
    }
    match (shared(), config_dir()) {
        (Some(policy), Some(_)) => env_for(&policy),
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const POLICY: &str = r#"{ "version": 1, "default_grant": "read", "grants": {
        "read": { "tier": "read", "mint": { "kind": "host_profile", "profile": "work" }, "agents": ["*"] },
        "deploy": { "tier": "write", "mint": { "kind": "command", "argv": ["/opt/escalate"] }, "agents": ["jcont"] } } }"#;

    #[test]
    fn the_config_is_written_in_one_folder_for_every_agent() {
        let root = tempfile::tempdir().unwrap();
        let policy = Policy::parse(POLICY).unwrap();
        let dir = render_into(root.path(), &policy).unwrap();
        assert_eq!(dir, root.path().join("container-aws"));
        let text = std::fs::read_to_string(dir.join("config")).unwrap();
        assert!(text.contains("[default]") && text.contains("[profile deploy]") && !text.contains("/opt/escalate") && !text.contains("work"), "{text}");
    }

    #[test]
    fn a_changed_policy_rewrites_the_file_and_an_unchanged_one_leaves_it_alone() {
        let root = tempfile::tempdir().unwrap();
        let policy = Policy::parse(POLICY).unwrap();
        let dir = render_into(root.path(), &policy).unwrap();
        let file = dir.join("config");
        let first = std::fs::metadata(&file).unwrap().modified().unwrap();
        render_into(root.path(), &policy).unwrap();
        assert_eq!(std::fs::metadata(&file).unwrap().modified().unwrap(), first, "same text, not written again");
        let moved = Policy::parse(&POLICY.replace(r#""default_grant": "read","#, r#""default_grant": "read", "region": "eu-west-1","#)).unwrap();
        render_into(root.path(), &moved).unwrap();
        assert!(std::fs::read_to_string(&file).unwrap().contains("region = eu-west-1"), "a changed policy is written");
        assert!(std::fs::read_dir(&dir).unwrap().all(|e| !e.unwrap().file_name().to_string_lossy().starts_with('.')), "no temp file is left");
    }

    #[test]
    fn the_four_variables_point_the_sdk_at_the_mounted_config_and_nothing_else() {
        let env: std::collections::HashMap<_, _> = env_for(&Policy::parse(POLICY).unwrap()).into_iter().collect();
        assert_eq!(env["AWS_CONFIG_FILE"], "/opt/agentmux/aws/config");
        assert_eq!(env["AWS_SHARED_CREDENTIALS_FILE"], "/dev/null");
        assert_eq!(env["AWS_EC2_METADATA_DISABLED"], "true");
        assert_eq!(env["AWS_PROFILE"], "read");
        assert_eq!(env.len(), 4);
        let no_default = Policy::parse(r#"{ "version": 1, "grants": { "x": { "tier": "read", "mint": { "kind": "host_profile", "profile": "p" } } } }"#).unwrap();
        assert!(!env_for(&no_default).iter().any(|(k, _)| k == "AWS_PROFILE"), "no default grant, no AWS_PROFILE");
    }

    #[test]
    fn under_test_nothing_is_asked_of_the_host() {
        // The machine running the tests has its own policy, which is not asked: no folder, no variables.
        assert_eq!(config_dir(), None);
        assert!(exec_env().is_empty());
    }
}

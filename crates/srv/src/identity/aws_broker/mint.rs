// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Getting credentials on the host for a grant (spec §7). srv never runs a shell: a recipe is an argument vector, a
//! placeholder is substituted whole into one argument, and the program's output is parsed and dropped.

use std::future::Future;
use std::path::PathBuf;
use std::time::Duration;

use agentmux_common::win32::NoWindow;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use zeroize::Zeroizing;

use super::policy::{Grant, Mint};

/// How long a recipe may take: a cached login is instant, a fresh one is a round trip or two.
pub const RECIPE_TIMEOUT: Duration = Duration::from_secs(25);
/// What of a recipe's stderr reaches the person.
const STDERR_KEPT: usize = 400;
/// AWS accepts only these in a session tag, and at most 256 characters.
const REASON_MAX: usize = 256;

/// Credentials as `credential_process` prints them. The secret and the token are zeroed when dropped.
#[derive(Clone)]
pub struct Minted {
    pub access_key_id: Zeroizing<String>,
    pub secret_access_key: Zeroizing<String>,
    pub session_token: Zeroizing<String>,
    pub expiration: DateTime<Utc>,
}

impl std::fmt::Debug for Minted {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Minted").field("expiration", &self.expiration).finish_non_exhaustive()
    }
}

impl Minted {
    /// What the SDKs read.
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "Version": 1,
            "AccessKeyId": self.access_key_id.as_str(),
            "SecretAccessKey": self.secret_access_key.as_str(),
            "SessionToken": self.session_token.as_str(),
            "Expiration": self.expiration.format("%Y-%m-%dT%H:%M:%SZ").to_string(),
        })
    }
}

/// Why a mint failed, in the words the person is told (spec §10).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MintError {
    /// The AWS CLI (or the recipe's program) is not on this machine, or is too old.
    CliMissing(String),
    /// The host's own login has lapsed.
    LoginNeeded(String),
    /// The recipe ran and refused, or printed something that is not credentials.
    Refused(String),
    TookTooLong,
}

pub struct RunSpec {
    pub program: PathBuf,
    pub args: Vec<String>,
    /// Added to the clean environment.
    pub env: Vec<(String, String)>,
    pub timeout: Duration,
}

pub struct RunOutput {
    pub success: bool,
    pub stdout: Vec<u8>,
    pub stderr: String,
}

pub enum RunError {
    Spawn(String),
    TimedOut,
}

/// How a recipe's program is found and run: the real one below, a fake in the tests.
pub trait Runner: Send + Sync {
    fn which(&self, program: &str) -> Option<PathBuf>;
    fn run(&self, spec: RunSpec) -> impl Future<Output = Result<RunOutput, RunError>> + Send;
}

/// Cleaned for a session tag: letters, digits, spaces and `_.:/=+-@`, one line, at most [`REASON_MAX`] characters.
pub fn clean_reason(reason: &str) -> String {
    let kept: String = reason.chars().map(|c| if c.is_alphanumeric() || " _.:/=+-@".contains(c) { c } else { ' ' }).collect();
    let flat = kept.split_whitespace().collect::<Vec<_>>().join(" ");
    // Never starts with `-`: a program given `--reason <this>` must not read it as an option.
    flat.chars().take(REASON_MAX).collect::<String>().trim_matches(|c: char| c == '-' || c == ' ').to_string()
}

pub struct MintCtx<'a> {
    pub agent_slug: &'a str,
    pub grant: &'a Grant,
    /// Already cleaned ([`clean_reason`]).
    pub reason: &'a str,
    pub seconds: u64,
    pub now_ms: i64,
}

impl MintCtx<'_> {
    /// `<agent slug>-<grant>-<unix ms>`, the session name a cloud record shows; at most 64 characters, as AWS allows.
    pub fn session_name(&self) -> String {
        let name = format!("{}-{}-{}", self.agent_slug, self.grant.name, self.now_ms);
        let cut = name.len().saturating_sub(64);
        // The slug and grant are ASCII, so cutting at a byte offset is safe; the tail (the time) is what must survive.
        name[cut..].trim_start_matches('-').to_string()
    }
}

/// The recipe's program and arguments, with the placeholders filled in.
pub fn recipe_argv(ctx: &MintCtx<'_>) -> Vec<String> {
    match &ctx.grant.mint {
        Mint::HostProfile { profile } => {
            ["aws", "configure", "export-credentials", "--profile", profile.as_str(), "--format", "process"].iter().map(|s| s.to_string()).collect()
        }
        Mint::Command { argv } => argv
            .iter()
            .map(|arg| {
                arg.replace("{reason}", ctx.reason)
                    .replace("{seconds}", &ctx.seconds.to_string())
                    .replace("{session}", &ctx.session_name())
                    .replace("{agent}", ctx.agent_slug)
            })
            .collect(),
    }
}

#[derive(Deserialize)]
struct ProcessJson {
    #[serde(rename = "Version")]
    version: Option<u32>,
    #[serde(rename = "AccessKeyId")]
    access_key_id: Option<String>,
    #[serde(rename = "SecretAccessKey")]
    secret_access_key: Option<String>,
    #[serde(rename = "SessionToken")]
    session_token: Option<String>,
    #[serde(rename = "Expiration")]
    expiration: Option<String>,
}

/// The credentials in a recipe's stdout: the last non-empty line, which must be the JSON object (a tool may print a
/// banner first). Long-lived keys, with no session token or no expiry, are refused: nothing that does not expire
/// is ever handed to a container.
pub fn parse_process_json(stdout: &[u8], now: DateTime<Utc>) -> Result<Minted, String> {
    let text = String::from_utf8_lossy(stdout);
    let line = text.lines().map(str::trim).rfind(|l| !l.is_empty()).ok_or("it printed nothing")?;
    let parsed: ProcessJson = serde_json::from_str(line).map_err(|_| "what it printed is not credential JSON".to_string())?;
    if parsed.version != Some(1) {
        return Err("what it printed is not credential_process version 1".to_string());
    }
    let field = |v: Option<String>, name: &str| v.filter(|s| !s.is_empty()).ok_or_else(|| format!("its credentials have no {name}"));
    let expiration = field(parsed.expiration, "Expiration (a key that does not expire is never handed to a container)")?;
    let expiration = DateTime::parse_from_rfc3339(&expiration).map_err(|_| "its Expiration is not a time".to_string())?.with_timezone(&Utc);
    if expiration <= now {
        return Err("its credentials have already expired".to_string());
    }
    Ok(Minted {
        access_key_id: Zeroizing::new(field(parsed.access_key_id, "AccessKeyId")?),
        secret_access_key: Zeroizing::new(field(parsed.secret_access_key, "SecretAccessKey")?),
        session_token: Zeroizing::new(field(parsed.session_token, "SessionToken (a long-lived key is never handed to a container)")?),
        expiration,
    })
}

fn short(s: &str) -> String {
    let flat = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= STDERR_KEPT {
        flat
    } else {
        format!("{}…", flat.chars().take(STDERR_KEPT).collect::<String>())
    }
}

/// What the host's AWS CLI says when its login has lapsed.
fn login_lapsed(stderr: &str) -> bool {
    let s = stderr.to_lowercase();
    ["sso login", "sso session", "token has expired", "error loading sso token", "refresh token", "expiredtoken"].iter().any(|m| s.contains(m))
}

/// Run the grant's recipe and parse what it prints.
pub async fn mint<R: Runner>(runner: &R, ctx: &MintCtx<'_>, now: DateTime<Utc>) -> Result<Minted, MintError> {
    let argv = recipe_argv(ctx);
    let (program_name, args) = argv.split_first().ok_or_else(|| MintError::Refused("the recipe is empty".to_string()))?;
    let is_cli = matches!(ctx.grant.mint, Mint::HostProfile { .. });
    let Some(program) = runner.which(program_name) else {
        return Err(if is_cli {
            MintError::CliMissing("this computer has no AWS CLI (v2.9 or newer), which this grant uses".to_string())
        } else {
            MintError::CliMissing(format!("{program_name} isn't installed on this computer (or isn't at that path)"))
        });
    };
    let spec = RunSpec {
        program,
        args: args.to_vec(),
        env: vec![("AGENTMUX_AGENT_ID".to_string(), ctx.agent_slug.to_string())],
        timeout: RECIPE_TIMEOUT,
    };
    let out = match runner.run(spec).await {
        Ok(out) => out,
        Err(RunError::TimedOut) => return Err(MintError::TookTooLong),
        Err(RunError::Spawn(e)) => return Err(MintError::Refused(format!("couldn't run {program_name}: {e}"))),
    };
    if !out.success {
        let said = short(&out.stderr);
        if is_cli {
            let s = out.stderr.to_lowercase();
            if s.contains("invalid choice") || s.contains("unknown command") || s.contains("export-credentials") && s.contains("usage") {
                return Err(MintError::CliMissing("the AWS CLI here is too old (v2.9 or newer is needed)".to_string()));
            }
            if login_lapsed(&out.stderr) {
                return Err(MintError::LoginNeeded(said));
            }
        }
        return Err(MintError::Refused(if said.is_empty() { "it exited with an error and said nothing".to_string() } else { said }));
    }
    parse_process_json(&out.stdout, now).map_err(|why| MintError::Refused(format!("{program_name}: {why}")))
}

/// Runs the recipe as a child of srv with a clean environment.
pub struct HostRunner;

/// What the host's own AWS setup and a program on PATH need, and nothing of the agent's or srv's instance identity.
fn passes(name: &str) -> bool {
    const FIXED: [&str; 17] = [
        "PATH", "HOME", "USERPROFILE", "APPDATA", "LOCALAPPDATA", "SYSTEMROOT", "SystemRoot", "TEMP", "TMP", "TMPDIR", "USER", "USERNAME", "LANG", "LC_ALL",
        "HTTPS_PROXY", "HTTP_PROXY", "NO_PROXY",
    ];
    FIXED.contains(&name) || name.starts_with("AWS_") || name == "SSL_CERT_FILE" || name == "XDG_CONFIG_HOME"
}

impl Runner for HostRunner {
    fn which(&self, program: &str) -> Option<PathBuf> {
        which::which(program).ok()
    }

    async fn run(&self, spec: RunSpec) -> Result<RunOutput, RunError> {
        let mut cmd = tokio::process::Command::new(&spec.program);
        cmd.env_clear();
        for (k, v) in std::env::vars().filter(|(k, _)| passes(k)) {
            cmd.env(k, v);
        }
        for (k, v) in &spec.env {
            cmd.env(k, v);
        }
        cmd.args(&spec.args).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped()).kill_on_drop(true);
        cmd.no_window();
        match tokio::time::timeout(spec.timeout, cmd.output()).await {
            Ok(Ok(out)) => Ok(RunOutput { success: out.status.success(), stdout: out.stdout, stderr: String::from_utf8_lossy(&out.stderr).into_owned() }),
            Ok(Err(e)) => Err(RunError::Spawn(e.to_string())),
            Err(_) => Err(RunError::TimedOut),
        }
    }
}

#[cfg(test)]
#[path = "mint_tests.rs"]
pub(crate) mod tests;

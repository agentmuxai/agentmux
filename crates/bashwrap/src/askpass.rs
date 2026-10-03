// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! askpass mode: `ssh` runs `$SSH_ASKPASS "<prompt>"` and reads the answer on
//! stdout. When AgentMux runs `ssh` for an agent it points `SSH_ASKPASS` here
//! and sets `AGENTMUX_ASKPASS_SECRET` on that `ssh` only; this asks srv
//! (`POST /api/v1/askpass`), which shows the prompt to the user, and prints
//! their answer. The secret, not anything here, decides whose prompt it is
//! (srv's `backend::remote::askpass`).
//!
//! Recognised before clap runs: ssh passes the prompt as the only argument,
//! with no subcommand.

use std::time::Duration;

const ENV_SECRET: &str = "AGENTMUX_ASKPASS_SECRET";

/// Longer than srv waits for the user (two minutes), so srv's own answer, or
/// its "not answered", is what arrives.
const ANSWER_TIMEOUT: Duration = Duration::from_secs(150);

/// bashwrap's own subcommands: never an askpass call, even if the variable
/// was inherited by something run under that ssh.
const SUBCOMMANDS: &[&str] = &[
    "exec",
    "hook",
    "precompact",
    "sessionstart",
    "help",
    "--help",
    "-h",
    "--version",
    "-V",
];

/// The prompt, when this run is ssh asking: the secret is set and the
/// arguments are not a bashwrap subcommand.
pub fn askpass_prompt(secret: Option<&str>, args: &[String]) -> Option<String> {
    secret.filter(|s| !s.is_empty())?;
    if args
        .first()
        .is_some_and(|a| SUBCOMMANDS.contains(&a.as_str()))
    {
        return None;
    }
    Some(args.join(" "))
}

/// Run askpass mode if this is an askpass call: `Some(exit code)` (0 with the
/// answer on stdout, 1 when there is none), `None` to carry on as bashwrap.
pub fn run_if_asked() -> Option<i32> {
    let secret = std::env::var(ENV_SECRET).ok();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let prompt = askpass_prompt(secret.as_deref(), &args)?;
    let secret = secret.unwrap_or_default();
    let Some(client) = crate::mps_client::WpsClient::from_env() else {
        eprintln!("agentmux askpass: AGENTMUX_LOCAL_URL / AGENTMUX_AUTH_KEY not set");
        return Some(1);
    };
    let rt = match tokio::runtime::Runtime::new() {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("agentmux askpass: {e}");
            return Some(1);
        }
    };
    let body = serde_json::json!({ "secret": secret, "prompt": prompt });
    let answer = rt.block_on(client.post_json("/api/v1/askpass", None, &body, ANSWER_TIMEOUT));
    match answer
        .ok()
        .and_then(|v| v.get("answer").and_then(|a| a.as_str()).map(str::to_string))
    {
        Some(text) => {
            // ssh reads one line; the answer is never logged or echoed.
            println!("{text}");
            Some(0)
        }
        None => Some(1),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn ssh_asking_is_askpass_and_everything_else_is_bashwrap() {
        assert_eq!(
            askpass_prompt(Some("amxa_x"), &args(&["asaf@area54's password: "])),
            Some("asaf@area54's password: ".to_string())
        );
        assert_eq!(
            askpass_prompt(None, &args(&["asaf@area54's password: "])),
            None,
            "no secret, no askpass"
        );
        assert_eq!(askpass_prompt(Some(""), &args(&["password: "])), None);
        assert_eq!(
            askpass_prompt(Some("amxa_x"), &args(&["exec", "--tool-id", "t"])),
            None,
            "an inherited secret never turns a subcommand into askpass"
        );
        assert_eq!(askpass_prompt(Some("amxa_x"), &args(&["hook"])), None);
    }
}

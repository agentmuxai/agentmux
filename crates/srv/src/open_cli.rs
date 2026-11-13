// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `agentmux-srv open`: sign a browser in to a headless srv that serves its own UI.
//! SPEC_HEADLESS_SERVER_PACKAGE_AND_LOCAL_WEB_UI §3.
//!
//! It reads the key file (owner-only), asks srv for a one-time code, opens the browser at
//! `/auth/enter#<code>` (or prints the line with `--print`), then stays in the person's terminal:
//! it shows the check phrase of the session that code made and asks `[y/N]`. Only the session of
//! the code it minted can be confirmed, and there is no flag that answers for the person.
//!
//! Exit codes: 0 signed in, 1 not signed in (declined, expired, or nothing entered), 2 bad use,
//! 3 could not do it (no terminal, srv not reachable, key refused).

use std::io::IsTerminal;
use std::time::Duration;

use crate::config::{CliArgs, SrvCommand};

/// How long `open` waits for a browser to enter the code, and for the person to answer.
const STEP_LIMIT: Duration = Duration::from_secs(60);
const POLL: Duration = Duration::from_millis(500);

/// Runs `open` and exits when that is what srv was started as; returns for any other start.
pub(crate) async fn exit_if_requested() {
    let Ok(cli) = <CliArgs as clap::Parser>::try_parse_from(std::env::args_os()) else { return };
    let Some(SrvCommand::Open { port, key_file, print, origin }) = cli.command else { return };
    let code = match prepare(cli.wavedata.as_deref(), port, key_file, origin) {
        Ok(target) => {
            let mut terminal = Stdio;
            run(&HttpServer::new(&target), &target.origin, print, &mut terminal).await
        }
        Err(e) => {
            eprintln!("agentmux-srv open: {e}");
            3
        }
    };
    std::process::exit(code);
}

struct Target {
    port: u16,
    key: String,
    /// What the browser opens: srv's own address, or the declared tunnel end.
    origin: String,
}

fn prepare(wavedata: Option<&std::path::Path>, port: u16, key_file: Option<std::path::PathBuf>, origin: Option<String>) -> Result<Target, String> {
    let path = match key_file {
        Some(p) => p,
        None => crate::headless::key_file_path(wavedata)?,
    };
    let key = crate::headless::read_key_file(&path).map_err(|e| format!("{e} (is the server started here, with this data folder?)"))?;
    let origin = origin.unwrap_or_else(|| format!("http://127.0.0.1:{port}"));
    Ok(Target { port, key, origin })
}

/// The line the browser is opened at. The code is in the fragment, which is never sent.
pub(crate) fn sign_in_url(origin: &str, code: &str) -> String {
    format!("{}/auth/enter#{code}", origin.trim_end_matches('/'))
}

/// `y` or `yes` is yes; anything else, including a bare Enter, is no.
pub(crate) fn is_yes(answer: &str) -> bool {
    matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes")
}

/// What srv says about the code `open` minted.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Entry {
    Unused,
    Entered(String),
    Gone,
}

/// srv, as `open` talks to it (a trait so the flow is tested without a network).
pub(crate) trait Server {
    async fn mint_code(&self) -> Result<String, String>;
    async fn entry(&self, code: &str) -> Result<Entry, String>;
    /// True when a session of that code was waiting for the answer.
    async fn answer(&self, code: &str, accept: bool) -> Result<bool, String>;
}

/// The person's terminal.
pub(crate) trait Terminal {
    fn is_terminal(&self) -> bool;
    fn say(&mut self, line: &str);
    /// One line the person types, or `None` at the time limit, at the end of input or on Ctrl-C.
    async fn ask(&mut self, prompt: &str, limit: Duration) -> Option<String>;
    fn open_browser(&mut self, url: &str) -> bool;
}

pub(crate) async fn run<S: Server, T: Terminal>(server: &S, origin: &str, print: bool, terminal: &mut T) -> i32 {
    if !terminal.is_terminal() {
        terminal.say("agentmux-srv open: it needs a terminal, because you confirm the sign-in there.");
        return 3;
    }
    let code = match server.mint_code().await {
        Ok(code) => code,
        Err(e) => {
            terminal.say(&format!("agentmux-srv open: {e}"));
            return 3;
        }
    };
    let url = sign_in_url(origin, &code);
    if print {
        terminal.say("Open this line in your own browser within a minute (it works once):");
        terminal.say(&url);
    } else if terminal.open_browser(&url) {
        terminal.say("Opened your browser. The page shows a short phrase.");
    } else {
        terminal.say("Could not open a browser. Open this line in your own browser within a minute (it works once):");
        terminal.say(&url);
    }

    // Wait for a browser to enter the code.
    let started = tokio::time::Instant::now();
    let phrase = loop {
        match server.entry(&code).await {
            Ok(Entry::Entered(phrase)) => break phrase,
            Ok(Entry::Unused) if started.elapsed() < STEP_LIMIT => tokio::time::sleep(POLL).await,
            Ok(Entry::Unused | Entry::Gone) => {
                terminal.say("No browser entered the code in time. Run it again.");
                return 1;
            }
            Err(e) => {
                terminal.say(&format!("agentmux-srv open: {e}"));
                return 3;
            }
        }
    };

    let prompt = format!("A browser entered with \"{phrase}\". Is that the page you opened? [y/N] ");
    let answer = terminal.ask(&prompt, STEP_LIMIT).await;
    let accept = answer.as_deref().is_some_and(is_yes);
    match server.answer(&code, accept).await {
        Ok(true) if accept => {
            terminal.say("Signed in. The browser is open to AgentMux.");
            0
        }
        Ok(true) => {
            terminal.say("Discarded. If your browser did not show that phrase, someone else used the code. Run `agentmux-srv open --print` and paste the line into your own browser.");
            1
        }
        Ok(false) => {
            terminal.say("That sign-in was no longer waiting. Run it again.");
            1
        }
        Err(e) => {
            terminal.say(&format!("agentmux-srv open: {e}"));
            3
        }
    }
}

// ---- The real server and terminal ----

struct HttpServer {
    client: reqwest::Client,
    base: String,
    key: String,
}

impl HttpServer {
    fn new(target: &Target) -> HttpServer {
        // No proxy for a loopback address; the Host header is the address, as srv requires.
        let client = reqwest::Client::builder().no_proxy().timeout(Duration::from_secs(10)).build().expect("a plain http client");
        HttpServer { client, base: format!("http://127.0.0.1:{}", target.port), key: target.key.clone() }
    }

    async fn post(&self, path: &str, body: serde_json::Value) -> Result<serde_json::Value, String> {
        let resp = self
            .client
            .post(format!("{}{path}", self.base))
            .header(agentmux_common::AUTH_KEY_HEADER, &self.key)
            .json(&body)
            .send()
            .await
            .map_err(|_| format!("could not reach srv at {} (is it running, and is that its web port?)", self.base))?;
        if !resp.status().is_success() {
            return Err("srv did not accept this (the key file may belong to another server, or the address is wrong)".into());
        }
        resp.json().await.map_err(|_| "srv answered something unexpected".to_string())
    }
}

impl Server for HttpServer {
    async fn mint_code(&self) -> Result<String, String> {
        let v = self.post("/auth/code", serde_json::json!({})).await?;
        v["code"].as_str().map(str::to_string).ok_or_else(|| "srv answered no code".to_string())
    }

    async fn entry(&self, code: &str) -> Result<Entry, String> {
        let v = self.post("/auth/pending", serde_json::json!({ "code": code })).await?;
        Ok(match (v["state"].as_str(), v["phrase"].as_str()) {
            (Some("unused"), _) => Entry::Unused,
            (Some("entered"), Some(phrase)) => Entry::Entered(phrase.to_string()),
            _ => Entry::Gone,
        })
    }

    async fn answer(&self, code: &str, accept: bool) -> Result<bool, String> {
        let v = self.post("/auth/confirm", serde_json::json!({ "code": code, "accept": accept })).await?;
        Ok(v["found"].as_bool().unwrap_or(false))
    }
}

struct Stdio;

impl Terminal for Stdio {
    fn is_terminal(&self) -> bool {
        std::io::stdin().is_terminal() && std::io::stdout().is_terminal()
    }

    fn say(&mut self, line: &str) {
        println!("{line}");
    }

    async fn ask(&mut self, prompt: &str, limit: Duration) -> Option<String> {
        use std::io::Write;
        print!("{prompt}");
        let _ = std::io::stdout().flush();
        let read = tokio::task::spawn_blocking(|| {
            let mut line = String::new();
            match std::io::stdin().read_line(&mut line) {
                Ok(n) if n > 0 => Some(line),
                _ => None,
            }
        });
        tokio::select! {
            line = read => line.ok().flatten(),
            _ = tokio::time::sleep(limit) => None,
            _ = tokio::signal::ctrl_c() => None,
        }
    }

    fn open_browser(&mut self, url: &str) -> bool {
        use std::process::{Command, Stdio};
        let mut cmd = if cfg!(target_os = "macos") {
            let mut c = Command::new("open");
            c.arg(url);
            c
        } else if cfg!(windows) {
            let mut c = Command::new("rundll32");
            c.args(["url.dll,FileProtocolHandler", url]);
            c
        } else {
            let mut c = Command::new("xdg-open");
            c.arg(url);
            c
        };
        cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().is_ok()
    }
}

#[cfg(test)]
#[path = "open_cli_tests.rs"]
mod tests;

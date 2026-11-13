// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `agentmux-srv open`: sign a browser in to a headless srv that serves its own UI.
//! SPEC_HEADLESS_SERVER_PACKAGE_AND_LOCAL_WEB_UI §3.
//!
//! It reads the key file (owner-only) and finds the server's port beside it, asks the server to prove
//! it holds that key and is bound to the address `open` is talking to (it never sends the key: see
//! [`crate::server::local_proof`]), asks srv for a one-time code, opens the browser at
//! `/auth/enter#<code>` (or prints the line with `--print`), then stays in the person's terminal:
//! it shows the check phrase of the session that code made and asks `[y/N]`. Only the session of
//! the code it minted can be confirmed, and there is no flag that answers for the person.
//!
//! Exit codes: 0 signed in, 1 not signed in (declined, expired, or nothing entered), 2 bad use,
//! 3 could not do it (no terminal, srv not reachable, key refused).

use std::io::IsTerminal;
use std::time::Duration;

use crate::config::{CliArgs, SrvCommand};
use crate::server::local_proof;

/// How long `open` waits for a browser to enter the code, and for the person to answer.
const STEP_LIMIT: Duration = Duration::from_secs(60);
const POLL: Duration = Duration::from_millis(500);

/// Runs `open` or `logout` and exits when that is what srv was started as; returns for any other start.
pub(crate) async fn exit_if_requested() {
    let Ok(cli) = <CliArgs as clap::Parser>::try_parse_from(std::env::args_os()) else { return };
    let (name, port, key_file, print, origin) = match cli.command {
        Some(SrvCommand::Open { port, key_file, print, origin }) => ("open", port, key_file, print, origin),
        Some(SrvCommand::Logout { port, key_file }) => ("logout", port, key_file, false, None),
        _ => return,
    };
    let code = match prepare(cli.wavedata.as_deref(), port, key_file, origin) {
        Ok(target) => match HttpServer::connect(&target).await {
            Ok(server) if name == "logout" => logout(&server).await,
            Ok(server) => run(&server, &target.origin, print, &mut Stdio).await,
            Err(e) => {
                eprintln!("agentmux-srv {name}: {e}");
                3
            }
        },
        Err(e) => {
            eprintln!("agentmux-srv {name}: {e}");
            3
        }
    };
    std::process::exit(code);
}

/// End every browser session of the server (`logout`): 0 done, 3 could not.
async fn logout(server: &HttpServer) -> i32 {
    match server.post("/auth/logout", serde_json::json!({})).await {
        Ok(v) => {
            let n = v["revoked"].as_u64().unwrap_or(0);
            println!("Signed out {n} browser session{}.", if n == 1 { "" } else { "s" });
            0
        }
        Err(e) => {
            eprintln!("agentmux-srv logout: {e}");
            3
        }
    }
}

pub(crate) struct Target {
    port: u16,
    key: String,
    /// What the browser opens: srv's own address, or the declared tunnel end.
    origin: String,
}

pub(crate) fn prepare(wavedata: Option<&std::path::Path>, port: Option<u16>, key_file: Option<std::path::PathBuf>, origin: Option<String>) -> Result<Target, String> {
    // Before anything is read: a line that carries a live code is never opened at any other page.
    let declared = origin
        .map(|o| crate::headless::normalize_public_origin(&o).map_err(|e| format!("--origin {o:?}: {e}")))
        .transpose()?;
    let path = match key_file {
        Some(p) => p,
        None => crate::headless::key_file_path(wavedata)?,
    };
    let key = crate::headless::read_key_file(&path).map_err(|e| format!("{e} (is the server started here, with this data folder?)"))?;
    let port = match port {
        Some(p) => p,
        None => crate::headless::read_port_file(&path)?,
    };
    let origin = declared.unwrap_or_else(|| format!("http://127.0.0.1:{port}"));
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

/// srv over HTTP, on a channel srv has proved it holds the key on.
pub(crate) struct HttpServer {
    client: reqwest::Client,
    base: String,
    bound: String,
    key: String,
    origin: String,
    channel: std::sync::Mutex<local_proof::Client>,
}

impl HttpServer {
    /// Reach the server at `127.0.0.1:<port>` and have it prove, before anything else is sent, that it
    /// holds this key and is bound to exactly that address. The key is never sent, to this listener or
    /// any other; a listener that is not this key's server sees one random nonce.
    pub(crate) async fn connect(target: &Target) -> Result<HttpServer, String> {
        // No proxy for a loopback address; the Host header is the address, as srv requires.
        let client = reqwest::Client::builder().no_proxy().timeout(Duration::from_secs(10)).build().expect("a plain http client");
        let bound = format!("127.0.0.1:{}", target.port);
        let base = format!("http://{bound}");
        let channel = Self::hello(&client, &base, &bound, &target.key, &target.origin).await?;
        Ok(HttpServer { client, base, bound, key: target.key.clone(), origin: target.origin.clone(), channel: std::sync::Mutex::new(channel) })
    }

    /// One hello: a channel srv has proved itself on, at the address `open` is talking to.
    async fn hello(client: &reqwest::Client, base: &str, bound: &str, key: &str, origin: &str) -> Result<local_proof::Client, String> {
        let nonce = local_proof::new_nonce();
        let resp = client
            .post(format!("{base}/auth/hello"))
            .json(&serde_json::json!({ "nonce": nonce }))
            .send()
            .await
            .map_err(|_| format!("could not reach srv at {base} (is it running, and is that its web port?)"))?;
        let not_this_server = || format!("the listener at {bound} did not prove it holds this key file's key, so nothing was sent to it: it is not the server this key file belongs to (check the port, or whether the server restarted)");
        if !resp.status().is_success() {
            return Err(not_this_server());
        }
        let v: serde_json::Value = resp.json().await.map_err(|_| not_this_server())?;
        let text = |name: &str| v[name].as_str().map(str::to_string);
        let (Some(channel), Some(srv_nonce), Some(declared), Some(proof)) = (text("channel"), text("nonce"), text("declared"), text("proof")) else {
            return Err(not_this_server());
        };
        let channel = local_proof::Client::accept(key.as_bytes(), &nonce, bound, local_proof::HelloReply { channel, nonce: srv_nonce, declared, proof })
            .map_err(|_| not_this_server())?;
        // The browser is opened only at an address this server proved is its own.
        if origin != base && channel.declared() != Some(origin) {
            return Err(format!("--origin {origin} is not an address this server accepts (it accepts {base}{})", channel.declared().map(|d| format!(" and {d}")).unwrap_or_default()));
        }
        Ok(channel)
    }

    /// One signed request, sent once: the status, the answer's proof, the answer and the request's number.
    async fn send(&self, path: &str, bytes: &[u8]) -> Result<(reqwest::StatusCode, Option<String>, Vec<u8>, u64), String> {
        let signed = self.channel.lock().unwrap_or_else(|e| e.into_inner()).sign("POST", path, bytes);
        let resp = self
            .client
            .post(format!("{}{path}", self.base))
            .header("content-type", "application/json")
            .header(local_proof::CHANNEL_HEADER, &signed.channel)
            .header(local_proof::SEQ_HEADER, signed.seq.to_string())
            .header(local_proof::PROOF_HEADER, &signed.proof)
            .body(bytes.to_vec())
            .send()
            .await
            .map_err(|_| format!("could not reach srv at {} (is it running, and is that its web port?)", self.base))?;
        let status = resp.status();
        let proof = resp.headers().get(local_proof::PROOF_HEADER).and_then(|v| v.to_str().ok()).map(str::to_string);
        let answer = resp.bytes().await.map_err(|_| "srv answered something unexpected".to_string())?;
        Ok((status, proof, answer.to_vec(), signed.seq))
    }

    async fn post(&self, path: &str, body: serde_json::Value) -> Result<serde_json::Value, String> {
        let bytes = serde_json::to_vec(&body).map_err(|e| e.to_string())?;
        let mut begun_again = false;
        loop {
            let (status, proof, answer, seq) = self.send(path, &bytes).await?;
            if !status.is_success() {
                // The first request on a channel can find it replaced, when other callers' hellos filled srv's
                // table between this one and the request: begin once more, then give up.
                if seq == 1 && !begun_again {
                    begun_again = true;
                    let fresh = Self::hello(&self.client, &self.base, &self.bound, &self.key, &self.origin).await?;
                    *self.channel.lock().unwrap_or_else(|e| e.into_inner()) = fresh;
                    continue;
                }
                return Err("srv did not accept this (it may have restarted: run `agentmux-srv open` again)".into());
            }
            let proved = proof.is_some_and(|p| self.channel.lock().unwrap_or_else(|e| e.into_inner()).answer_ok(seq, status.as_u16(), &answer, &p));
            if !proved {
                return Err("srv's answer did not carry the proof of the key, so it was not used".into());
            }
            return serde_json::from_slice(&answer).map_err(|_| "srv answered something unexpected".to_string());
        }
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
        use agentmux_common::win32::NoWindow;
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
        // The browser needs nothing of this instance's identity or key.
        crate::backend::pane_env::sanitize_external_std_command(&mut cmd);
        cmd.no_window().stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().is_ok()
    }
}

#[cfg(test)]
#[path = "open_cli_tests.rs"]
mod tests;

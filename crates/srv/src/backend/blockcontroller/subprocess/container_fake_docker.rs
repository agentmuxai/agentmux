// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A fake Docker daemon for the container turn's tests: just enough of the Engine API over a local TCP
//! socket for `ContainerManager` (built on a `bollard` client pointed at it) to run a turn without a
//! runtime on the machine. It answers the exec calls by what the command is (the uid probe, the memory
//! counter, the prompt cleanup, the interrupt, the turn itself), takes the prompt archive, and records
//! every call in order so a test can say what the turn did to the container and in which sequence.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

/// What an exec is for, told from its command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    IdProbe,
    Oom,
    RemovePrompt,
    Interrupt,
    Turn,
    Other,
}

/// One call the daemon received.
#[derive(Debug, Clone)]
pub enum Call {
    Exec { kind: Kind, cmd: Vec<String>, env: Vec<String> },
    Upload { tar: Vec<u8> },
    Inspect,
}

/// What the daemon does for one turn's exec.
#[derive(Debug, Clone, Default)]
pub struct Turn {
    /// `(1 = stdout | 2 = stderr, bytes)`, written in order.
    pub frames: Vec<(u8, Vec<u8>)>,
    /// The exit code an inspect reports; `None` reports none at all.
    pub exit: Option<i64>,
    /// An inspect reports the exec as still running.
    pub still_running: bool,
    /// After the frames, half a frame header and then the connection drops.
    pub cut_mid_frame: bool,
    /// After the frames, the stream stays open until the client lets go.
    pub hold_open: bool,
    /// Creating the exec is refused with a server error.
    pub refuse: bool,
}

impl Turn {
    /// A turn whose stdout is `lines` (one frame each) and that exits with `code`.
    pub fn says(lines: &[&str], code: i64) -> Self {
        Turn {
            frames: lines.iter().map(|l| (1u8, format!("{l}\n").into_bytes())).collect(),
            exit: Some(code),
            ..Default::default()
        }
    }
}

#[derive(Default)]
pub struct State {
    pub calls: Vec<Call>,
    /// The scripts for the turns that come, in order; a turn past the end exits 0 without a word.
    pub turns: VecDeque<Turn>,
    /// The uid probe is refused.
    pub id_probe_fails: bool,
    /// The prompt archive is refused.
    pub upload_fails: bool,
    /// What the memory counter reads at each probe, in order; none left reads nothing.
    pub oom_counts: VecDeque<u64>,
    execs: Vec<ExecRec>,
}

struct ExecRec {
    kind: Kind,
    turn: Option<Turn>,
    oom: Option<u64>,
}

pub struct FakeDocker {
    pub state: Arc<Mutex<State>>,
    port: u16,
}

impl FakeDocker {
    pub async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind the fake daemon");
        let port = listener.local_addr().unwrap().port();
        let state = Arc::new(Mutex::new(State::default()));
        let served = state.clone();
        tokio::spawn(async move {
            loop {
                let Ok((sock, _)) = listener.accept().await else { return };
                tokio::spawn(serve(sock, served.clone()));
            }
        });
        FakeDocker { state, port }
    }

    pub fn manager(&self) -> crate::backend::container::ContainerManager {
        let docker = bollard::Docker::connect_with_http(&format!("http://127.0.0.1:{}", self.port), 30, bollard::API_DEFAULT_VERSION)
            .expect("a client for the fake daemon");
        crate::backend::container::ContainerManager::with_docker(docker, None)
    }

    pub fn script(&self, turns: Vec<Turn>) {
        self.state.lock().unwrap().turns = turns.into();
    }

    /// The calls so far as short labels: `exec:turn`, `upload`, `inspect`.
    pub fn labels(&self) -> Vec<String> {
        self.state
            .lock()
            .unwrap()
            .calls
            .iter()
            .map(|c| match c {
                Call::Exec { kind, .. } => format!("exec:{}", format!("{kind:?}").to_lowercase()),
                Call::Upload { .. } => "upload".to_string(),
                Call::Inspect => "inspect".to_string(),
            })
            .collect()
    }

    /// The commands of the execs of one kind, in order.
    pub fn execs(&self, wanted: Kind) -> Vec<(Vec<String>, Vec<String>)> {
        self.state
            .lock()
            .unwrap()
            .calls
            .iter()
            .filter_map(|c| match c {
                Call::Exec { kind, cmd, env } if *kind == wanted => Some((cmd.clone(), env.clone())),
                _ => None,
            })
            .collect()
    }

    /// The prompt files uploaded, each as `(name, mode, uid, text)`.
    pub fn uploads(&self) -> Vec<(String, u32, u64, String)> {
        let mut out = Vec::new();
        for call in self.state.lock().unwrap().calls.iter() {
            if let Call::Upload { tar, .. } = call {
                let mut archive = tar::Archive::new(tar.as_slice());
                for entry in archive.entries().unwrap() {
                    let mut entry = entry.unwrap();
                    let name = entry.path().unwrap().to_string_lossy().into_owned();
                    let (mode, uid) = (entry.header().mode().unwrap(), entry.header().uid().unwrap());
                    let mut text = String::new();
                    std::io::Read::read_to_string(&mut entry, &mut text).unwrap();
                    out.push((name, mode, uid, text));
                }
            }
        }
        out
    }
}

fn kind_of(cmd: &[String]) -> Kind {
    let at = |i: usize| cmd.get(i).map(String::as_str).unwrap_or("");
    if at(0) == "pkill" {
        Kind::Interrupt
    } else if at(0) == "bash" && at(3) == "agentmux-turn" {
        Kind::Turn
    } else if at(0) == "sh" && at(2) == "id -u; id -g" {
        Kind::IdProbe
    } else if at(0) == "sh" && at(2).contains("memory.events") {
        Kind::Oom
    } else if at(0) == "sh" && at(3) == "agentmux-cleanup" {
        Kind::RemovePrompt
    } else {
        Kind::Other
    }
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

/// One request: `(method, path without the API version, query, body)`.
async fn read_request(sock: &mut TcpStream, buf: &mut Vec<u8>) -> Option<(String, String, String, Vec<u8>)> {
    loop {
        if let Some(end) = find(buf, b"\r\n\r\n") {
            let head = String::from_utf8_lossy(&buf[..end]).to_string();
            let len = head
                .lines()
                .find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap_or(0)))
                .unwrap_or(0);
            let total = end + 4 + len;
            while buf.len() < total {
                let mut chunk = [0u8; 8192];
                let n = sock.read(&mut chunk).await.ok()?;
                if n == 0 {
                    return None;
                }
                buf.extend_from_slice(&chunk[..n]);
            }
            let body = buf[end + 4..total].to_vec();
            buf.drain(..total);
            let mut first = head.lines().next()?.split_whitespace();
            let (method, target) = (first.next()?.to_string(), first.next()?.to_string());
            let (path, query) = target.split_once('?').unwrap_or((&target, ""));
            let path = match path.strip_prefix("/v") {
                Some(rest) if rest.starts_with(|c: char| c.is_ascii_digit()) => rest.find('/').map(|i| &rest[i..]).unwrap_or("/"),
                _ => path,
            };
            return Some((method, path.to_string(), query.to_string(), body));
        }
        let mut chunk = [0u8; 8192];
        let n = sock.read(&mut chunk).await.ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&chunk[..n]);
    }
}

fn reply(status: &str, body: &str) -> Vec<u8> {
    format!("HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}", body.len()).into_bytes()
}

fn frame(kind: u8, data: &[u8]) -> Vec<u8> {
    let mut out = vec![kind, 0, 0, 0];
    out.extend((data.len() as u32).to_be_bytes());
    out.extend_from_slice(data);
    out
}

async fn serve(mut sock: TcpStream, state: Arc<Mutex<State>>) {
    let mut buf = Vec::new();
    while let Some((method, path, _query, body)) = read_request(&mut sock, &mut buf).await {
        let parts: Vec<&str> = path.trim_matches('/').split('/').collect();
        let answer = match (method.as_str(), parts.as_slice()) {
            ("POST", ["containers", _name, "exec"]) => create_exec(&state, &body),
            ("POST", ["exec", id, "start"]) => {
                // The connection is the exec's output stream from here on, and ends with it.
                start_exec(&mut sock, &state, id).await;
                return;
            }
            ("GET", ["exec", id, "json"]) => inspect_exec(&state, id),
            ("PUT", ["containers", _name, "archive"]) => take_archive(&state, body),
            _ => reply("404 Not Found", r#"{"message":"not part of the fake daemon"}"#),
        };
        if sock.write_all(&answer).await.is_err() {
            return;
        }
    }
}

fn exec_index(id: &str) -> usize {
    id.trim_start_matches("exec-").parse().unwrap_or(usize::MAX)
}

/// `POST /containers/{name}/exec`: record the command and answer with an exec id, or refuse.
fn create_exec(state: &Mutex<State>, body: &[u8]) -> Vec<u8> {
    let spec: serde_json::Value = serde_json::from_slice(body).unwrap_or_default();
    let strings = |key: &str| -> Vec<String> {
        spec[key].as_array().map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect()).unwrap_or_default()
    };
    let (cmd, env) = (strings("Cmd"), strings("Env"));
    let kind = kind_of(&cmd);
    let mut st = state.lock().unwrap();
    st.calls.push(Call::Exec { kind, cmd, env });
    let mut rec = ExecRec { kind, turn: None, oom: None };
    let mut refuse = kind == Kind::IdProbe && st.id_probe_fails;
    if kind == Kind::Turn {
        let turn = st.turns.pop_front().unwrap_or(Turn { exit: Some(0), ..Default::default() });
        refuse = turn.refuse;
        rec.turn = Some(turn);
    }
    if kind == Kind::Oom {
        rec.oom = st.oom_counts.pop_front();
    }
    if refuse {
        return reply("500 Internal Server Error", r#"{"message":"the fake daemon refuses this exec"}"#);
    }
    st.execs.push(rec);
    reply("201 Created", &format!(r#"{{"Id":"exec-{}"}}"#, st.execs.len() - 1))
}

/// `POST /exec/{id}/start`: switch the connection to the exec's output stream and write what the
/// script says, then close it (or hold it open, or cut it mid-frame).
async fn start_exec(sock: &mut TcpStream, state: &Mutex<State>, id: &str) {
    let (kind, turn, oom) = {
        let st = state.lock().unwrap();
        let Some(rec) = st.execs.get(exec_index(id)) else { return };
        (rec.kind, rec.turn.clone(), rec.oom)
    };
    let mut out = b"HTTP/1.1 101 UPGRADED\r\nConnection: Upgrade\r\nUpgrade: tcp\r\n\r\n".to_vec();
    let mut hold = false;
    match kind {
        Kind::IdProbe => out.extend(frame(1, b"1000\n1000\n")),
        Kind::Oom => {
            if let Some(n) = oom {
                out.extend(frame(1, format!("low 0\nhigh 0\nmax 0\noom 0\noom_kill {n}\n").as_bytes()));
            }
        }
        Kind::Turn => {
            let turn = turn.unwrap_or_default();
            for (k, data) in &turn.frames {
                out.extend(frame(*k, data));
            }
            hold = turn.hold_open;
            if turn.cut_mid_frame {
                out.extend_from_slice(&[1, 0, 0, 0]);
            }
        }
        _ => {}
    }
    if sock.write_all(&out).await.is_err() {
        return;
    }
    if hold {
        let mut sink = [0u8; 64];
        while matches!(sock.read(&mut sink).await, Ok(n) if n > 0) {}
    }
    let _ = sock.shutdown().await;
}

/// `GET /exec/{id}/json`: the exit code the script gives the exec.
fn inspect_exec(state: &Mutex<State>, id: &str) -> Vec<u8> {
    let mut st = state.lock().unwrap();
    st.calls.push(Call::Inspect);
    let turn = st.execs.get(exec_index(id)).and_then(|r| r.turn.clone());
    match turn {
        Some(t) if t.still_running => reply("200 OK", &format!(r#"{{"ID":"{id}","Running":true}}"#)),
        Some(Turn { exit: Some(code), .. }) => reply("200 OK", &format!(r#"{{"ID":"{id}","Running":false,"ExitCode":{code}}}"#)),
        Some(_) => reply("200 OK", &format!(r#"{{"ID":"{id}","Running":false}}"#)),
        None => reply("200 OK", &format!(r#"{{"ID":"{id}","Running":false,"ExitCode":0}}"#)),
    }
}

/// `PUT /containers/{name}/archive`: keep the tar, or refuse it.
fn take_archive(state: &Mutex<State>, body: Vec<u8>) -> Vec<u8> {
    let mut st = state.lock().unwrap();
    st.calls.push(Call::Upload { tar: body });
    if st.upload_fails {
        reply("500 Internal Server Error", r#"{"message":"the fake daemon refuses this archive"}"#)
    } else {
        reply("200 OK", "{}")
    }
}

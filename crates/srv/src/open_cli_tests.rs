// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use std::sync::Mutex;
use std::time::Instant;

use super::*;
use crate::server::local_proof::Channels;
use crate::server::local_session::{CodeState, Entered, SessionStore};

/// `open` talking to a real session store.
#[derive(Default)]
struct Local {
    store: SessionStore,
    entered: Mutex<Vec<Entered>>,
}

impl Local {
    /// A browser enters the code; a spent code makes nothing.
    fn enter(&self, code: &str) {
        if let Some(e) = self.store.enter(code, Instant::now()) {
            self.entered.lock().unwrap().push(e);
        }
    }
}

impl Server for Local {
    async fn mint_code(&self) -> Result<String, String> {
        Ok(self.store.mint_code(Instant::now()))
    }
    async fn entry(&self, code: &str) -> Result<Entry, String> {
        Ok(match self.store.code_state(code, Instant::now()) {
            CodeState::Unused => Entry::Unused,
            CodeState::Entered { phrase } => Entry::Entered(phrase),
            CodeState::Gone => Entry::Gone,
        })
    }
    async fn answer(&self, code: &str, accept: bool) -> Result<bool, String> {
        Ok(self.store.decide(code, accept, Instant::now()))
    }
}

/// A terminal that types a fixed answer, and a browser that enters the code of the line it is given.
struct Script<'a> {
    server: Option<&'a Local>,
    /// Another local user read the line from the process list and enters the code first.
    thief_first: bool,
    tty: bool,
    answer: Option<&'static str>,
    said: Vec<String>,
    asked: Vec<String>,
    opened: Vec<String>,
}

impl<'a> Script<'a> {
    fn new(server: &'a Local, answer: Option<&'static str>) -> Script<'a> {
        Script { server: Some(server), thief_first: false, tty: true, answer, said: vec![], asked: vec![], opened: vec![] }
    }

    /// The person's browser is given this line (opened for them, or pasted from `--print`).
    fn browser_gets(&self, line: &str) {
        let (Some(server), Some((_, code))) = (self.server, line.split_once("/auth/enter#")) else { return };
        if self.thief_first {
            server.enter(code);
        }
        server.enter(code);
    }
}

impl Terminal for Script<'_> {
    fn is_terminal(&self) -> bool {
        self.tty
    }
    fn say(&mut self, line: &str) {
        if line.starts_with("http://") {
            self.browser_gets(line);
        }
        self.said.push(line.to_string());
    }
    async fn ask(&mut self, prompt: &str, _limit: Duration) -> Option<String> {
        self.asked.push(prompt.to_string());
        self.answer.map(str::to_string)
    }
    fn open_browser(&mut self, url: &str) -> bool {
        self.browser_gets(url);
        self.opened.push(url.to_string());
        true
    }
}

const ORIGIN: &str = "http://127.0.0.1:7380";

#[test]
fn the_url_keeps_the_code_in_the_fragment() {
    assert_eq!(sign_in_url(ORIGIN, "abc"), "http://127.0.0.1:7380/auth/enter#abc");
    assert_eq!(sign_in_url("http://127.0.0.1:8080/", "abc"), "http://127.0.0.1:8080/auth/enter#abc");
}

#[test]
fn only_y_or_yes_confirms_and_a_bare_enter_does_not() {
    for yes in ["y", "Y", "yes", "YES", " y \n", "y\r\n"] {
        assert!(is_yes(yes), "{yes:?}");
    }
    for no in ["", "\n", "n", "N", "no", "yy", "yes please", "ok", " "] {
        assert!(!is_yes(no), "{no:?}");
    }
}

#[tokio::test]
async fn confirming_signs_the_browser_in() {
    let server = Local::default();
    let mut t = Script::new(&server, Some("y\n"));
    assert_eq!(run(&server, ORIGIN, false, &mut t).await, 0);
    assert!(t.opened[0].starts_with("http://127.0.0.1:7380/auth/enter#"));
    let phrase = server.entered.lock().unwrap()[0].phrase.clone();
    assert!(t.asked[0].contains(&format!("\"{phrase}\"")) && t.asked[0].ends_with("[y/N] "), "{:?}", t.asked);
    assert_eq!(server.store.session_count(Instant::now()), 1);
    assert!(t.said.last().unwrap().starts_with("Signed in"));
}

#[tokio::test]
async fn anything_but_a_yes_discards_the_session() {
    for answer in [Some("\n"), Some("n\n"), Some("maybe\n"), None] {
        let server = Local::default();
        let mut t = Script::new(&server, answer);
        assert_eq!(run(&server, ORIGIN, false, &mut t).await, 1, "{answer:?}");
        assert_eq!(server.store.session_count(Instant::now()), 0, "{answer:?}");
        assert!(t.said.last().unwrap().contains("--print"), "{:?}", t.said);
    }
}

/// Another browser entered the code first: `open` shows that session's phrase, which the person's
/// own browser (refused, the code is spent) never showed. Answering `n` leaves nothing live.
#[tokio::test]
async fn a_session_the_person_does_not_recognise_is_discarded_and_nothing_goes_live() {
    let server = Local::default();
    let mut t = Script::new(&server, Some("n\n"));
    t.thief_first = true;
    assert_eq!(run(&server, ORIGIN, false, &mut t).await, 1);
    assert_eq!(server.entered.lock().unwrap().len(), 1, "the person's own entry made nothing");
    assert_eq!(server.store.session_count(Instant::now()), 0);
}

#[tokio::test]
async fn print_shows_the_line_and_opens_no_browser() {
    let server = Local::default();
    let mut t = Script::new(&server, Some("y\n"));
    assert_eq!(run(&server, ORIGIN, true, &mut t).await, 0);
    assert!(t.opened.is_empty());
    assert!(t.said.iter().any(|l| l.starts_with("http://127.0.0.1:7380/auth/enter#")), "{:?}", t.said);
}

#[tokio::test]
async fn no_terminal_means_no_sign_in() {
    let server = Local::default();
    let mut t = Script::new(&server, Some("y\n"));
    t.tty = false;
    assert_eq!(run(&server, ORIGIN, false, &mut t).await, 3);
    assert!(t.opened.is_empty() && t.asked.is_empty());
}

/// A code nobody enters: `open` gives up after the minute and says so.
#[tokio::test(start_paused = true)]
async fn a_code_nobody_enters_ends_the_wait_after_a_minute() {
    let server = Local::default();
    let mut t = Script::new(&server, Some("y\n"));
    t.server = None; // the browser never opens the line
    let before = tokio::time::Instant::now();
    assert_eq!(run(&server, ORIGIN, false, &mut t).await, 1);
    assert!(tokio::time::Instant::now() - before >= STEP_LIMIT);
    assert!(t.asked.is_empty());
    assert!(t.said.last().unwrap().contains("No browser entered"));
}

/// The typed origin is the one `--public-origin` accepts, or nothing: a line carrying a live code is
/// never opened at another page. It is judged before the key file is read.
#[test]
fn the_origin_is_held_to_the_rule_of_the_servers_public_origin() {
    let missing = std::path::PathBuf::from("no/such/key-file");
    for bad in ["https://127.0.0.1:8080", "http://localhost:8080", "http://evil.test", "http://127.0.0.1", "http://192.0.2.1:80", "http://127.0.0.1:8080/x"] {
        let err = prepare(None, Some(7380), Some(missing.clone()), Some(bad.to_string())).err().expect(bad);
        assert!(err.contains("--origin"), "{bad}: {err}");
    }
    let dir = tempfile::tempdir().unwrap();
    let key = dir.path().join("k");
    std::fs::write(&key, "secret\n").unwrap();
    let t = prepare(None, Some(7380), Some(key.clone()), Some("http://127.1:8080/".into())).unwrap();
    assert_eq!(t.origin, "http://127.0.0.1:8080");
    let t = prepare(None, Some(7380), Some(key), None).unwrap();
    assert_eq!(t.origin, "http://127.0.0.1:7380");
}

// ---- The port: written beside the key file by the server ----

#[test]
fn the_port_is_read_beside_the_key_file_and_the_flag_overrides_it() {
    let dir = tempfile::tempdir().unwrap();
    let key = dir.path().join("srv-auth-key");
    std::fs::write(&key, "secret\n").unwrap();
    assert_eq!(crate::headless::port_file_path(&key), dir.path().join("srv-auth-key.port"));

    let err = prepare(None, None, Some(key.clone()), None).err().expect("no port file yet");
    assert!(err.contains("--port"), "{err}");
    std::fs::write(dir.path().join("srv-auth-key.port"), "7391\n").unwrap();
    let t = prepare(None, None, Some(key.clone()), None).unwrap();
    assert_eq!((t.port, t.origin.as_str()), (7391, "http://127.0.0.1:7391"));
    assert_eq!(prepare(None, Some(7392), Some(key.clone()), None).unwrap().port, 7392, "--port wins");
    for junk in ["", "abc", "0", "70000", "-1"] {
        std::fs::write(dir.path().join("srv-auth-key.port"), junk).unwrap();
        assert!(prepare(None, None, Some(key.clone()), None).is_err(), "{junk:?}");
    }
}

// ---- Proving the server before anything is sent ----

const SECRET: &str = "the-key-that-must-never-be-sent";

fn target(port: u16, origin: Option<&str>) -> Target {
    Target { port, key: SECRET.to_string(), origin: origin.map(str::to_string).unwrap_or_else(|| format!("http://127.0.0.1:{port}")) }
}

/// A listener that is not srv: it answers every request with a hello-shaped answer, and keeps what it was sent.
async fn impostor() -> (u16, std::sync::Arc<Mutex<Vec<u8>>>) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let seen = std::sync::Arc::new(Mutex::new(Vec::new()));
    let log = seen.clone();
    tokio::spawn(async move {
        while let Ok((mut conn, _)) = listener.accept().await {
            let log = log.clone();
            tokio::spawn(async move {
                let mut buf = vec![0u8; 8192];
                // Read the request (headers and the small body), then answer.
                let mut got = 0;
                while let Ok(n) = conn.read(&mut buf[got..]).await {
                    got += n;
                    let text = String::from_utf8_lossy(&buf[..got]).to_string();
                    if n == 0 || (text.contains("\r\n\r\n") && text.contains("nonce")) {
                        break;
                    }
                }
                log.lock().unwrap().extend_from_slice(&buf[..got]);
                let body = format!(r#"{{"channel":"{0}","nonce":"{0}","declared":"","proof":"{1}"}}"#, "a".repeat(64), "0".repeat(64));
                let reply = format!("HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}", body.len());
                let _ = conn.write_all(reply.as_bytes()).await;
            });
        }
    });
    (port, seen)
}

/// A server that holds `SECRET` and answers a hello for the address it is bound to.
async fn real_srv(declared: &'static str) -> u16 {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let channels = std::sync::Arc::new(Channels::default());
    let app = axum::Router::new().route(
        "/auth/hello",
        axum::routing::post(move |axum::Json(v): axum::Json<serde_json::Value>| {
            let channels = channels.clone();
            async move {
                let h = channels.hello(SECRET.as_bytes(), v["nonce"].as_str().unwrap_or_default(), &format!("127.0.0.1:{port}"), declared, Instant::now()).unwrap();
                axum::Json(serde_json::json!({ "channel": h.channel, "nonce": h.nonce, "declared": declared, "proof": h.proof }))
            }
        }),
    );
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    port
}

/// A forwarder: connections to its port are passed on, byte for byte, to `to`.
async fn forwarder(to: u16) -> u16 {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        while let Ok((mut a, _)) = listener.accept().await {
            tokio::spawn(async move {
                if let Ok(mut b) = tokio::net::TcpStream::connect(("127.0.0.1", to)).await {
                    let _ = tokio::io::copy_bidirectional(&mut a, &mut b).await;
                }
            });
        }
    });
    port
}

#[tokio::test]
async fn a_listener_that_is_not_srv_gets_no_key_and_no_usable_proof() {
    let (port, seen) = impostor().await;
    let err = HttpServer::connect(&target(port, None)).await.err().expect("not srv");
    assert!(err.contains("did not prove") && err.contains("nothing was sent"), "{err}");
    let sent = String::from_utf8_lossy(&seen.lock().unwrap()).to_lowercase();
    assert!(sent.contains("post /auth/hello"), "{sent}");
    assert!(!sent.contains(&SECRET.to_lowercase()), "the key reached the listener");
    for header in ["x-authkey", crate::server::local_proof::PROOF_HEADER, crate::server::local_proof::CHANNEL_HEADER] {
        assert!(!sent.contains(header), "{header} reached the listener: {sent}");
    }
    assert!(!sent.contains("/auth/code"), "nothing but the hello was asked");
}

#[tokio::test]
async fn nothing_listening_is_a_plain_line() {
    let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let err = HttpServer::connect(&target(port, None)).await.err().unwrap();
    assert!(err.contains("could not reach srv"), "{err}");
}

#[tokio::test]
async fn the_real_server_is_accepted_and_a_forwarder_on_another_port_is_not() {
    let srv = real_srv("").await;
    assert!(HttpServer::connect(&target(srv, None)).await.is_ok());
    let fwd = forwarder(srv).await;
    let err = HttpServer::connect(&target(fwd, None)).await.err().expect("a forwarder");
    assert!(err.contains("did not prove"), "{err}");
}

#[tokio::test]
async fn the_browser_is_opened_only_at_an_address_the_server_proved() {
    let srv = real_srv("http://127.0.0.1:9191").await;
    assert!(HttpServer::connect(&target(srv, Some("http://127.0.0.1:9191"))).await.is_ok(), "the declared origin");
    let err = HttpServer::connect(&target(srv, Some("http://127.0.0.1:9292"))).await.err().expect("another origin");
    assert!(err.contains("--origin") && err.contains("http://127.0.0.1:9191"), "{err}");
    let bare = real_srv("").await;
    assert!(HttpServer::connect(&target(bare, Some("http://127.0.0.1:9191"))).await.is_err(), "nothing declared");
}

/// What srv does on a proof-only route: admit the request or refuse it, and sign the answer.
fn admit_and_answer(channels: &Mutex<Channels>, path: &str, headers: &axum::http::HeaderMap, body: &axum::body::Bytes, answer: serde_json::Value) -> axum::response::Response {
    use axum::response::IntoResponse;
    let get = |n: &str| headers.get(n).and_then(|v| v.to_str().ok()).unwrap_or_default().to_string();
    let seq = get(crate::server::local_proof::SEQ_HEADER).parse::<u64>().unwrap_or(0);
    let admitted = channels.lock().unwrap().admit(&get(crate::server::local_proof::CHANNEL_HEADER), seq, "POST", path, body, &get(crate::server::local_proof::PROOF_HEADER), Instant::now());
    match admitted {
        None => axum::http::StatusCode::FORBIDDEN.into_response(),
        Some(a) => {
            let answer = serde_json::to_vec(&answer).unwrap();
            let proof = crate::server::local_proof::response_proof(&a.key, a.seq, 200, &answer);
            ([(crate::server::local_proof::PROOF_HEADER, proof)], answer).into_response()
        }
    }
}

/// A server whose first `lose` channels are replaced before their first request arrives (other
/// callers' hellos filled its table in between). Counts the hellos it answered.
async fn flooded_srv(lose: usize) -> (u16, std::sync::Arc<std::sync::atomic::AtomicUsize>) {
    use axum::body::Bytes;
    use axum::http::{HeaderMap, StatusCode};
    use std::sync::atomic::{AtomicUsize, Ordering};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let channels = std::sync::Arc::new(Mutex::new(Channels::default()));
    let hellos = std::sync::Arc::new(AtomicUsize::new(0));
    let (c1, h1, c2, c3) = (channels.clone(), hellos.clone(), channels.clone(), channels.clone());
    let app = axum::Router::new()
        .route(
            "/auth/hello",
            axum::routing::post(move |axum::Json(v): axum::Json<serde_json::Value>| {
                let (channels, hellos) = (c1.clone(), h1.clone());
                async move {
                    let n = hellos.fetch_add(1, Ordering::SeqCst);
                    let mut store = channels.lock().unwrap();
                    let h = store.hello(SECRET.as_bytes(), v["nonce"].as_str().unwrap_or_default(), &format!("127.0.0.1:{port}"), "", Instant::now()).unwrap();
                    if n < lose {
                        *store = Channels::default();
                    }
                    axum::Json(serde_json::json!({ "channel": h.channel, "nonce": h.nonce, "declared": "", "proof": h.proof }))
                }
            }),
        )
        .route("/auth/code", axum::routing::post(move |headers: HeaderMap, body: Bytes| {
            let channels = c2.clone();
            async move { admit_and_answer(&channels, "/auth/code", &headers, &body, serde_json::json!({ "code": "abc" })) }
        }))
        .route("/auth/logout", axum::routing::post(move |headers: HeaderMap, body: Bytes| {
            let channels = c3.clone();
            async move { admit_and_answer(&channels, "/auth/logout", &headers, &body, serde_json::json!({ "revoked": 3 })) }
        }));
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (port, hellos)
}

/// The first request on a new channel finds it replaced: `open` begins once more, and only once.
#[tokio::test]
async fn open_begins_again_once_when_its_first_request_is_refused() {
    let (port, hellos) = flooded_srv(1).await;
    let server = HttpServer::connect(&target(port, None)).await.unwrap();
    assert_eq!(server.mint_code().await, Ok("abc".to_string()));
    assert_eq!(hellos.load(std::sync::atomic::Ordering::SeqCst), 2);

    let (port, hellos) = flooded_srv(2).await;
    let server = HttpServer::connect(&target(port, None)).await.unwrap();
    let err = server.mint_code().await.unwrap_err();
    assert!(err.contains("did not accept"), "{err}");
    assert_eq!(hellos.load(std::sync::atomic::Ordering::SeqCst), 2, "one more hello, no more");
}

/// `logout` proves the key like `open` and reports how many sessions the server ended.
#[tokio::test]
async fn logout_signs_every_browser_out_and_says_how_many() {
    let (port, _) = flooded_srv(0).await;
    let server = HttpServer::connect(&target(port, None)).await.unwrap();
    assert_eq!(logout(&server).await, 0);
    let v = server.post("/auth/logout", serde_json::json!({})).await.unwrap();
    assert_eq!(v["revoked"], 3);
    let (gone, _) = (std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port(), ());
    assert!(HttpServer::connect(&target(gone, None)).await.is_err(), "nothing to log out of");
}

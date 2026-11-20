// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The question asked before a quit that would stop agents in the middle of a turn.
//!
//! Quitting stops every agent. A window's own close warns first (`frontend/app/window/window-close.ts`);
//! a quit that comes from outside a window does not pass a page. So whoever starts such a quit asks
//! srv who is working (`GET /agentmux/quit-question`) and, when srv says to ask, puts the question in
//! a native dialog of its own: one that needs no window and that no page or agent can answer.
//!
//! The launcher asks for the tray's Quit (`crates/launcher/src/quit_ask.rs`); the host asks for the
//! macOS app menu's Quit and Cmd+Q (`crates/cef/src/macos_menu.rs`). They share this: the question,
//! the text, and the macOS dialog.
//!
//! Quitting must always work. No answer from srv within [`ANSWER_WITHIN`] (srv is gone, hung or from
//! a version without the question), or no way to show a dialog, quits as before. Only an explicit
//! "keep running" stops the quit.

use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

/// srv's route (`crates/srv/src/server/quit_question.rs`).
const QUESTION_PATH: &str = "/agentmux/quit-question";

/// How long srv has to answer, for the connection and for each read.
const ANSWER_WITHIN: Duration = Duration::from_millis(1500);

pub const DIALOG_TITLE: &str = "Quit AgentMux";
#[cfg_attr(not(any(target_os = "linux", target_os = "macos", test)), allow(dead_code))]
pub const QUIT: &str = "Quit";
#[cfg_attr(not(any(target_os = "linux", target_os = "macos", test)), allow(dead_code))]
pub const KEEP_RUNNING: &str = "Keep running";

/// What srv said when it said to ask.
#[derive(Debug, PartialEq, Eq)]
pub struct Question {
    /// The agents in the middle of a turn, by name.
    pub working: Vec<String>,
    /// How many agents the quit stops.
    pub agents: u64,
}

/// srv's answer as a question to put, or `None` when it says not to ask or says nothing usable.
fn parse(json: &str) -> Option<Question> {
    let answer: serde_json::Value = serde_json::from_str(json).ok()?;
    if !answer.get("ask")?.as_bool()? {
        return None;
    }
    let working: Vec<String> =
        answer.get("working")?.as_array()?.iter().filter_map(|n| n.as_str()).map(str::to_string).collect();
    if working.is_empty() {
        return None;
    }
    let agents = answer.get("agents").and_then(|n| n.as_u64()).unwrap_or(0).max(working.len() as u64);
    Some(Question { working, agents })
}

/// The JSON object in an HTTP response with status 200, whatever the body's framing.
fn json_of(response: &str) -> Option<&str> {
    let (head, body) = response.split_once("\r\n\r\n")?;
    if !head.starts_with("HTTP/1.1 200") {
        return None;
    }
    let (start, end) = (body.find('{')?, body.rfind('}')?);
    (start < end).then(|| &body[start..=end])
}

/// Ask srv. Blocking, bounded by [`ANSWER_WITHIN`] for the connection and for each read.
pub fn ask_srv(web_endpoint: &str, auth_key: &str) -> Option<Question> {
    let addr = web_endpoint.to_socket_addrs().ok()?.next()?;
    let mut stream = TcpStream::connect_timeout(&addr, ANSWER_WITHIN).ok()?;
    stream.set_read_timeout(Some(ANSWER_WITHIN)).ok()?;
    stream.set_write_timeout(Some(ANSWER_WITHIN)).ok()?;
    let request =
        format!("GET {QUESTION_PATH} HTTP/1.1\r\nHost: {web_endpoint}\r\nX-AuthKey: {auth_key}\r\nConnection: close\r\n\r\n");
    stream.write_all(request.as_bytes()).ok()?;
    let mut response = String::new();
    stream.read_to_string(&mut response).ok()?;
    parse(json_of(&response)?)
}

/// "Alpha, Beta and 3 more", as the window's close warning names them.
fn names_line(names: &[String]) -> String {
    const MAX: usize = 4;
    match names {
        [] => String::new(),
        [one] => one.clone(),
        _ if names.len() <= MAX => format!("{} and {}", names[..names.len() - 1].join(", "), names[names.len() - 1]),
        _ => format!("{} and {} more", names[..MAX].join(", "), names.len() - MAX),
    }
}

/// The dialog's text.
pub fn text(q: &Question) -> String {
    let verb = if q.working.len() == 1 { "is" } else { "are" };
    let agents = if q.agents == 1 { "1 agent".to_string() } else { format!("{} agents", q.agents) };
    format!(
        "{} {verb} working. Quit AgentMux?\n\n\
         This stops {agents}. Your windows open again at the next launch.",
        names_line(&q.working)
    )
}

/// Whether the quit goes ahead: always, unless there is a question and the person answers it with
/// "keep running". `confirm` is `None` when no dialog could be shown.
pub fn goes_ahead(question: Option<&Question>, confirm: impl FnOnce(&str) -> Option<bool>) -> bool {
    match question {
        None => true,
        Some(q) => confirm(&text(q)).unwrap_or(true),
    }
}

/// What `osascript` said. The Quit button exits 0 and prints its name. "Keep running" is the
/// dialog's cancel button, so it (and Escape) exits non-zero with AppleScript's "User canceled"
/// error, -128. Any other failure is a dialog that was never answered: `None`.
#[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
pub fn macos_answer(success: bool, stdout: &str, stderr: &str) -> Option<bool> {
    if success && stdout.contains(&format!("button returned:{QUIT}")) {
        Some(true)
    } else if stderr.contains("(-128)") || (success && stdout.contains(&format!("button returned:{KEEP_RUNNING}"))) {
        Some(false)
    } else {
        None
    }
}

/// The macOS question as an alert, for `osascript -l JavaScript`: the arguments are the text, the Quit
/// and Keep running titles and the icon's path (empty for none), and it prints `button returned:<title>`
/// as `display dialog` did, so [`macos_answer`] reads either. Keep running is the first button: the
/// default (Return), and the answer for anything that is not a click on Quit. It is an alert and not
/// `display dialog` so that it carries the app's own icon. Its keys are the ones `display dialog` had:
/// Return, and the mouse (Tab and Space reach Quit only with the system's Full Keyboard Access on).
/// Escape, and a key for Quit, do not work in an alert shown by `osascript` (tried on macOS 26: no key
/// equivalent on the Quit button fires, and a button titled Cancel gets Escape only now and then and
/// loses Return and the default highlight).
#[cfg(any(target_os = "macos", test))]
const MACOS_ALERT: &str = r#"ObjC.import('AppKit');
function run(argv) {
  var body = argv[0], quitTitle = argv[1], keepTitle = argv[2], iconPath = argv[3] || '';
  var app = $.NSApplication.sharedApplication;
  app.setActivationPolicy($.NSApplicationActivationPolicyAccessory);
  var alert = $.NSAlert.alloc.init;
  alert.alertStyle = $.NSAlertStyleWarning;
  var parts = body.split('\n\n');
  alert.messageText = parts[0];
  alert.informativeText = parts.slice(1).join('\n\n');
  if (iconPath !== '') {
    try {
      var img = $.NSImage.alloc.initWithContentsOfFile(iconPath);
      if (img) { alert.icon = img; }
    } catch (e) { }
  }
  alert.addButtonWithTitle(keepTitle);
  alert.addButtonWithTitle(quitTitle);
  app.activateIgnoringOtherApps(true);
  var response = (typeof alert.runModal === 'function') ? alert.runModal() : alert.runModal;
  return 'button returned:' + (response == 1001 ? quitTitle : keepTitle);
}"#;

/// The app's icon, for the alert: `Contents/Resources/AgentMux.icns` of the `.app` that `exe` is in
/// (`package-macos.sh` writes it there), when `exists` says it is there. A build run from a folder has
/// none, and the alert then carries the system's default icon.
#[cfg(any(target_os = "macos", test))]
fn app_icon(exe: &std::path::Path, exists: impl Fn(&std::path::Path) -> bool) -> Option<std::path::PathBuf> {
    let mut bundle = std::path::PathBuf::new();
    for part in exe.components() {
        bundle.push(part);
        if part.as_os_str().to_string_lossy().ends_with(".app") {
            let icon = bundle.join("Contents").join("Resources").join("AgentMux.icns");
            return exists(&icon).then_some(icon);
        }
    }
    None
}

/// macOS: ask in a native alert whose default is to keep running, through `osascript` (as the
/// upgrade question does). The text goes to it as arguments, never into the script, so nothing in
/// it is parsed. `Ok(true)`: quit. `Ok(false)`: keep running. `Err`: no dialog was answered, and
/// why, for the caller's log; the quit then goes ahead.
#[cfg(target_os = "macos")]
pub fn confirm_macos(body: &str) -> Result<bool, String> {
    let icon = std::env::current_exe()
        .ok()
        .and_then(|exe| app_icon(&exe, |p| p.is_file()))
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    let out = std::process::Command::new("/usr/bin/osascript")
        .args(["-l", "JavaScript", "-e", MACOS_ALERT])
        .arg(body)
        .arg(QUIT)
        .arg(KEEP_RUNNING)
        .arg(icon)
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|e| format!("couldn't open osascript to ask: {e}"))?;
    macos_answer(
        out.status.success(),
        &String::from_utf8_lossy(&out.stdout),
        &String::from_utf8_lossy(&out.stderr),
    )
    .ok_or_else(|| format!("osascript did not show the question ({})", out.status))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|n| n.to_string()).collect()
    }

    #[test]
    fn srvs_answer_is_a_question_only_when_it_says_to_ask_and_names_someone() {
        assert_eq!(
            parse(r#"{"ask":true,"working":["Korp","Loap"],"agents":5}"#),
            Some(Question { working: names(&["Korp", "Loap"]), agents: 5 })
        );
        assert_eq!(parse(r#"{"ask":false,"working":["Korp"],"agents":1}"#), None, "the warning is off");
        assert_eq!(parse(r#"{"ask":false,"working":[],"agents":3}"#), None, "nobody is mid-turn");
        assert_eq!(parse(r#"{"ask":true,"working":[],"agents":3}"#), None);
        assert_eq!(parse(r#"{"error":"not found"}"#), None, "a srv without the question");
        assert_eq!(parse("not json"), None);
        // A count that is missing or too small never says fewer agents than are named.
        assert_eq!(parse(r#"{"ask":true,"working":["Korp","Loap"]}"#).map(|q| q.agents), Some(2));
    }

    #[test]
    fn the_json_is_taken_from_a_200_whatever_the_framing() {
        let plain = "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: 14\r\n\r\n{\"ask\":false}";
        assert_eq!(json_of(plain), Some("{\"ask\":false}"));
        let chunked = "HTTP/1.1 200 OK\r\ntransfer-encoding: chunked\r\n\r\nd\r\n{\"ask\":false}\r\n0\r\n\r\n";
        assert_eq!(json_of(chunked), Some("{\"ask\":false}"));
        assert_eq!(json_of("HTTP/1.1 404 Not Found\r\n\r\n{\"error\":\"x\"}"), None);
        assert_eq!(json_of("HTTP/1.1 200 OK\r\n\r\n"), None);
        assert_eq!(json_of("garbage"), None);
        assert_eq!(json_of("HTTP/1.1 200 OK\r\n\r\n}{"), None);
    }

    #[test]
    fn the_text_names_who_is_working_and_how_many_agents_stop() {
        let one = text(&Question { working: names(&["Korp"]), agents: 1 });
        assert!(one.starts_with("Korp is working. Quit AgentMux?\n\nThis stops 1 agent. "), "{one}");
        let two = text(&Question { working: names(&["Korp", "Loap"]), agents: 5 });
        assert!(two.starts_with("Korp and Loap are working. Quit AgentMux?\n\nThis stops 5 agents. "), "{two}");
        assert_eq!(names_line(&names(&["A", "B", "C"])), "A, B and C");
        assert_eq!(names_line(&names(&["A", "B", "C", "D", "E", "F"])), "A, B, C, D and 2 more");
    }

    #[test]
    fn only_an_explicit_keep_running_stops_the_quit() {
        let q = Question { working: names(&["Korp"]), agents: 2 };
        assert!(goes_ahead(None, |_| panic!("nothing to ask: no dialog")), "srv gave no question");
        assert!(goes_ahead(Some(&q), |_| Some(true)));
        assert!(!goes_ahead(Some(&q), |_| Some(false)));
        assert!(goes_ahead(Some(&q), |_| None), "no dialog could be shown: quitting still works");
    }

    #[test]
    fn a_macos_dialog_that_failed_is_not_an_answer_to_keep_running() {
        // The Quit button, the cancel button (error -128), and everything else.
        assert_eq!(macos_answer(true, "button returned:Quit\n", ""), Some(true));
        let cancelled = "0:140: execution error: User canceled. (-128)";
        assert_eq!(macos_answer(false, "", cancelled), Some(false));
        assert_eq!(macos_answer(true, "button returned:Keep running\n", ""), Some(false));
        assert_eq!(macos_answer(false, "", "execution error: No user interaction allowed. (-1713)"), None);
        assert_eq!(macos_answer(false, "", ""), None);
        assert_eq!(macos_answer(true, "", ""), None);
    }

    #[test]
    fn the_alert_prints_what_display_dialog_printed_and_keeps_running_unless_quit_was_clicked() {
        // `macos_answer` reads the alert's output as it read the old dialog's.
        assert!(MACOS_ALERT.contains("'button returned:' + (response == 1001 ? quitTitle : keepTitle)"));
        let printed = format!("button returned:{QUIT}\n");
        assert_eq!(macos_answer(true, &printed, ""), Some(true));
        // The first button (Keep running) is the default, the second is Quit.
        let keep_at = MACOS_ALERT.find("addButtonWithTitle(keepTitle)").expect("keep button");
        let quit_at = MACOS_ALERT.find("addButtonWithTitle(quitTitle)").expect("quit button");
        assert!(keep_at < quit_at, "Return must pick Keep running");
        // The text, the titles and the icon come as arguments, never as script text.
        assert!(!MACOS_ALERT.contains(KEEP_RUNNING) && !MACOS_ALERT.contains("AgentMux"), "no text in the script");
    }

    #[test]
    fn the_icon_is_the_one_in_the_app_the_program_runs_from() {
        use std::path::{Path, PathBuf};
        let in_app = Path::new("/Applications/AgentMux.app/Contents/MacOS/agentmux-launcher");
        let icon = PathBuf::from("/Applications/AgentMux.app/Contents/Resources/AgentMux.icns");
        assert_eq!(app_icon(in_app, |p| p == icon), Some(icon.clone()));
        assert_eq!(app_icon(in_app, |_| false), None, "an icon that is not there is not offered");
        // Inside a helper app of the bundle, the outer app's icon is still the one.
        let helper = Path::new("/Applications/AgentMux.app/Contents/Frameworks/AgentMux Helper.app/Contents/MacOS/AgentMux Helper");
        assert_eq!(app_icon(helper, |p| p == icon), Some(icon.clone()));
        // A build run from a folder has no app, so the system's icon stays.
        assert_eq!(app_icon(Path::new("/Users/me/agentmux/target/debug/agentmux-launcher"), |_| true), None);
    }

    #[test]
    fn srv_is_asked_over_http_with_the_key_and_no_answer_is_no_question() {
        use std::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = listener.local_addr().unwrap().to_string();
        let server = std::thread::spawn(move || {
            let (mut conn, _) = listener.accept().unwrap();
            let mut request = [0u8; 1024];
            let n = conn.read(&mut request).unwrap();
            let request = String::from_utf8_lossy(&request[..n]).to_string();
            let body = r#"{"ask":true,"working":["Korp"],"agents":2}"#;
            let _ = write!(conn, "HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}", body.len());
            request
        });
        assert_eq!(ask_srv(&endpoint, "the-key"), Some(Question { working: names(&["Korp"]), agents: 2 }));
        let request = server.join().unwrap();
        assert!(request.starts_with("GET /agentmux/quit-question HTTP/1.1\r\n"), "{request}");
        assert!(request.contains("X-AuthKey: the-key\r\n"), "{request}");
        // Nothing listens there any more: no question, so the quit goes ahead.
        assert_eq!(ask_srv(&endpoint, "the-key"), None);
    }
}

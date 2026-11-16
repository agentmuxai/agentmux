// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The helper side of the icon protocol ([`super::pool`]): read one request per
//! line from stdin, ask the system, answer on stdout. Ends when stdin closes,
//! which is when srv goes away or kills it.

use std::io::{BufRead, Write};

use serde::Deserialize;

use super::pool::ERR_NO_ICON;
use super::request::IconRequest;

#[derive(Deserialize)]
struct ChildRequest {
    kind: String,
    ext: String,
    px: u32,
}

/// The picture for `req`, or the error word the answer carries.
#[cfg(windows)]
fn render(req: &IconRequest) -> Result<Vec<u8>, &'static str> {
    use super::shell_windows::{render, ShellError};
    render(req).map_err(|e| match e {
        ShellError::NoIcon => ERR_NO_ICON,
        ShellError::Failed(why) => why,
    })
}

#[cfg(not(windows))]
fn render(_req: &IconRequest) -> Result<Vec<u8>, &'static str> {
    Err("unsupported")
}

/// Answer one request line. Every request is validated again here: this process
/// does what the pipe says, so the pipe is not trusted either.
fn answer(line: &str) -> Vec<u8> {
    let parsed = serde_json::from_str::<ChildRequest>(line)
        .map_err(|_| "bad-request")
        .and_then(|r| IconRequest::parse(Some(&r.kind), Some(&r.ext), Some(&r.px.to_string())).map_err(|_| "bad-request"))
        .and_then(|req| render(&req));
    match parsed {
        Ok(png) => {
            let mut out = format!("{{\"ok\":true,\"len\":{}}}\n", png.len()).into_bytes();
            out.extend_from_slice(&png);
            out
        }
        Err(err) => format!("{{\"ok\":false,\"err\":\"{err}\"}}\n").into_bytes(),
    }
}

/// Run as the helper; the process's exit code.
pub fn run_child() -> i32 {
    #[cfg(windows)]
    if let Err(e) = super::shell_windows::init() {
        eprintln!("file icons helper: {e}");
        return 1;
    }
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout().lock();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { return 0 };
        if stdout.write_all(&answer(&line)).and_then(|()| stdout.flush()).is_err() {
            return 0;
        }
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_malformed_or_out_of_range_request_is_refused_without_touching_the_system() {
        for bad in [
            "",
            "not json",
            r#"{"kind":"file","ext":"rs"}"#,
            r#"{"kind":"file","ext":"..\x","px":16}"#,
            r#"{"kind":"file","ext":"rs","px":17}"#,
            r#"{"kind":"drive","ext":"","px":16}"#,
        ] {
            assert_eq!(answer(bad), b"{\"ok\":false,\"err\":\"bad-request\"}\n".to_vec(), "{bad}");
        }
    }

    #[cfg(windows)]
    #[test]
    fn a_good_request_is_answered_with_a_header_line_and_that_many_png_bytes() {
        let _turn = super::super::shell_windows::SHELL_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        super::super::shell_windows::init().unwrap();
        let out = answer(r#"{"kind":"file","ext":"txt","px":16}"#);
        let nl = out.iter().position(|b| *b == b'\n').unwrap();
        let header: serde_json::Value = serde_json::from_slice(&out[..nl]).unwrap();
        assert_eq!(header["ok"], true);
        assert_eq!(header["len"].as_u64().unwrap() as usize, out.len() - nl - 1);
        assert_eq!(&out[nl + 1..nl + 5], b"\x89PNG");
    }
}

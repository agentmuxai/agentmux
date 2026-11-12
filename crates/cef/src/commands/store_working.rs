// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Who "Restart now" on a Microsoft Store update would interrupt (task #2514): the agents in the middle of a turn,
//! by name, for the question the Maintenance row asks before it installs. srv knows
//! (`GET /agentmux/quit-question`, `crates/srv/src/server/quit_question.rs`, the same answer the tray's Quit and the
//! window's close warning use); this reads it and says nothing else. Any failure is "nobody named": the row then
//! asks without names, because the restart still stops every agent.

use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

use serde::Serialize;

/// srv's route.
const QUESTION_PATH: &str = "/agentmux/quit-question";

/// How long srv has to answer, for the connection and for each read.
const ANSWER_WITHIN: Duration = Duration::from_millis(1500);

/// What the row names.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize)]
pub struct Working {
    /// The agents in the middle of a turn, by name.
    pub working: Vec<String>,
    /// How many agents a restart stops.
    pub agents: u64,
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

/// srv's answer as who is working; nothing usable is nobody.
fn parse(json: &str) -> Working {
    let Ok(answer) = serde_json::from_str::<serde_json::Value>(json) else { return Working::default() };
    let working: Vec<String> = answer
        .get("working")
        .and_then(|w| w.as_array())
        .map(|names| names.iter().filter_map(|n| n.as_str()).map(str::to_string).collect())
        .unwrap_or_default();
    let agents = answer.get("agents").and_then(|n| n.as_u64()).unwrap_or(0).max(working.len() as u64);
    Working { working, agents }
}

/// Ask srv. Blocking, bounded by [`ANSWER_WITHIN`] for the connection and for each read.
pub fn fetch(web_endpoint: &str, auth_key: &str) -> Working {
    let ask = || -> Option<Working> {
        let addr = web_endpoint.to_socket_addrs().ok()?.next()?;
        let mut stream = TcpStream::connect_timeout(&addr, ANSWER_WITHIN).ok()?;
        stream.set_read_timeout(Some(ANSWER_WITHIN)).ok()?;
        stream.set_write_timeout(Some(ANSWER_WITHIN)).ok()?;
        let request = format!("GET {QUESTION_PATH} HTTP/1.1\r\nHost: {web_endpoint}\r\nX-AuthKey: {auth_key}\r\nConnection: close\r\n\r\n");
        stream.write_all(request.as_bytes()).ok()?;
        let mut response = String::new();
        stream.read_to_string(&mut response).ok()?;
        Some(parse(json_of(&response)?))
    };
    ask().unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_names_come_from_srvs_answer_whether_or_not_it_says_to_ask() {
        let w = parse(r#"{"ask":false,"working":["Korp","Loap"],"agents":5}"#);
        assert_eq!((w.working.as_slice(), w.agents), (["Korp".to_string(), "Loap".to_string()].as_slice(), 5));
    }

    #[test]
    fn nothing_usable_is_nobody_named() {
        assert_eq!(parse("not json"), Working::default());
        assert_eq!(parse(r#"{"ask":true}"#), Working::default());
        let w = parse(r#"{"working":["Korp"]}"#);
        assert_eq!(w.agents, 1, "never fewer agents than the ones named");
    }

    #[test]
    fn only_a_200_response_is_read_whatever_the_framing() {
        assert_eq!(json_of("HTTP/1.1 200 OK\r\nX: y\r\n\r\n1f\r\n{\"agents\":2}\r\n0\r\n\r\n"), Some("{\"agents\":2}"));
        assert_eq!(json_of("HTTP/1.1 401 Unauthorized\r\n\r\n{\"agents\":2}"), None);
    }
}

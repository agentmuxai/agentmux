// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What `agentmux status` says of the browsers signed in to a server that serves its own UI: how many, and how each recent
//! confirm of a sign-in arrived (from a process outside every agent's process tree, from one that could not be found, or
//! refused because it came from inside an agent's tree). A line holds no code, cookie or token.

use crate::{run_client, DataPaths};

/// The answer of `agentmux-srv sessions`.
pub struct Browsers {
    pub count: u64,
    /// `<unix ms> <session's check phrase> <how>`, oldest first.
    pub confirms: Vec<String>,
}

/// How many browsers are signed in now and how the recent confirms arrived, asked of the running server; None when it cannot
/// say (the server does not serve its own UI, or does not answer).
pub fn asked(paths: &DataPaths) -> Option<Browsers> {
    let (code, out) = run_client(paths, "sessions", &[], true);
    if code != 0 {
        return None;
    }
    parse(&out)
}

/// The number on the first line, then a `confirm <unix ms> <phrase> <how>` line for each confirm.
pub fn parse(out: &str) -> Option<Browsers> {
    let mut lines = out.lines();
    let count = lines.next()?.trim().parse().ok()?;
    let confirms = lines.filter_map(|l| l.trim().strip_prefix("confirm ").map(str::to_string)).collect();
    Some(Browsers { count, confirms })
}

/// What `status` says of the browsers signed in.
pub fn line(n: u64) -> String {
    match n {
        0 => "none signed in (`agentmux open` signs one in)".to_string(),
        n => format!("{n} signed in"),
    }
}

/// The lines of one confirm, as `status` prints it.
pub fn confirm_line(entry: &str) -> String {
    let mut parts = entry.splitn(3, ' ');
    let (at, session, how) = (parts.next().unwrap_or(""), parts.next().unwrap_or("?"), parts.next().unwrap_or("?"));
    let when = at.parse::<u64>().ok().map(|ms| agentmux_common::time::now_secs_u64().saturating_sub(ms / 1000));
    let ago = when.map_or_else(|| "at an unknown time".to_string(), |s| format!("{} ago", crate::human_duration(s)));
    format!("    sign-in {session}: confirmed {ago}, {how}")
}

pub fn print(b: &Browsers) {
    println!("  browsers: {}", line(b.count));
    for entry in &b.confirms {
        println!("{}", confirm_line(entry));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_number_then_each_confirm() {
        let out = "2\nconfirm 1700000000000 kela-mivo peer outside agent trees\nconfirm 1700000005000 sano-teri refused: agent:opaz\n";
        let b = parse(out).unwrap();
        assert_eq!(b.count, 2);
        assert_eq!(b.confirms, ["1700000000000 kela-mivo peer outside agent trees", "1700000005000 sano-teri refused: agent:opaz"]);
        assert!(confirm_line(&b.confirms[1]).contains("sano-teri") && confirm_line(&b.confirms[1]).ends_with("refused: agent:opaz"));
    }

    #[test]
    fn a_server_that_lists_no_confirms_still_gives_the_number() {
        assert_eq!(parse("0\n").unwrap().count, 0);
        assert!(parse("0\n").unwrap().confirms.is_empty());
        assert!(parse("not a number").is_none());
    }

    #[test]
    fn the_browsers_line_says_none_or_how_many() {
        assert!(line(0).contains("none") && line(0).contains("agentmux open"));
        assert_eq!(line(1), "1 signed in");
        assert_eq!(line(3), "3 signed in");
    }
}

// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The names no agent may have: ones that read as the person using AgentMux or as AgentMux itself.
//!
//! One list ([`RESERVED`]), one check ([`check`]). Every way an agent comes to exist or to be called
//! something else (create, spawn, rename, define, import, fork) refuses a reserved id, slug or display
//! name with [`check_all`]. An agent that already has such a name is left alone (see [`reserved_in`]).
//!
//! A name is compared case-insensitively, after trimming, with look-alike characters folded to the letter
//! they imitate (fullwidth forms, the Cyrillic and Greek letters that copy Latin ones, and the digits and
//! signs that stand in for letters), and with separators dropped: `Op3rator`, `оperator` (Cyrillic о),
//! `O.P.E.R.A.T.O.R` and `operator-2` all read as `operator`.

/// Words that read as the person or the system, and the names AgentMux uses for its own senders.
pub const RESERVED: &[&str] = &[
    // The person.
    "operator", "user", "human", "person", "owner", "admin", "administrator", "root",
    // The system.
    "system", "agentmux", "unproven", "unknown", "window", "host", "srv",
    // The names AgentMux already uses for its own senders and notes.
    "githubconsumer", "muxreview", "reagent", "swarmforward", "swarm", "supervisor", "broadcast", "bulletin",
    "jekt", "cron", "drone", "slack", "discord", "telegram", "whatsapp",
    // The words the records use for the person and the system as actors.
    "you", "vault", "failover", "scheduler", "watchdog", "loop", "armory",
];

/// A letter a look-alike character stands for, or the character itself lower-cased.
fn fold(c: char) -> char {
    let c = match c as u32 {
        0xFF01..=0xFF5E => char::from_u32(c as u32 - 0xFEE0).unwrap_or(c),
        _ => c,
    };
    match c.to_lowercase().next().unwrap_or(c) {
        // Cyrillic and Greek letters that copy Latin ones.
        'а' | 'α' => 'a',
        'е' | 'є' | 'ε' => 'e',
        'о' | 'ο' | 'σ' => 'o',
        'р' | 'ρ' => 'p',
        'с' | 'ϲ' => 'c',
        'у' | 'γ' => 'y',
        'х' | 'χ' => 'x',
        'і' | 'ї' | 'ι' => 'i',
        'ѕ' => 's',
        'ј' => 'j',
        'ԁ' => 'd',
        'ո' | 'ν' => 'n',
        'τ' => 't',
        'μ' | 'м' => 'm',
        'н' | 'η' => 'h',
        'к' | 'κ' => 'k',
        'т' => 't',
        'в' | 'β' => 'b',
        // Digits and signs that stand in for letters.
        '0' => 'o',
        '1' | '|' | '!' => 'i',
        '3' => 'e',
        '4' | '@' => 'a',
        '5' | '$' => 's',
        '7' => 't',
        '8' => 'b',
        other => other,
    }
}

/// The letters of `name` as a reader takes them: look-alikes folded, everything that is not a letter dropped.
fn letters(name: &str) -> String {
    name.trim()
        .chars()
        .filter(|c| !matches!(*c, '\u{200B}'..='\u{200F}' | '\u{2060}'..='\u{2064}' | '\u{FEFF}' | '\u{00AD}' | '\u{202A}'..='\u{202E}'))
        .map(fold)
        .filter(|c| c.is_alphabetic())
        .collect()
}

/// The reserved word `name` reads as, or `None`: the whole name, or that word with a counting number after
/// it (`admin2`, `user-3`).
pub fn check(name: &str) -> Option<&'static str> {
    // The name as typed, and without a counting number or separators after it.
    let counted = name.trim().trim_end_matches(|c: char| c.is_ascii_digit() || !c.is_alphanumeric());
    [letters(name), letters(counted)]
        .iter()
        .filter(|read| !read.is_empty())
        .find_map(|read| RESERVED.iter().copied().find(|word| read == word))
}

/// What an id, slug or display name is refused with: the name is reserved, and what to do instead.
pub fn refusal(name: &str, word: &str) -> String {
    format!("the name \"{name}\" is reserved (it reads as \"{word}\"): pick a nickname for the agent, a name that reads as a person's")
}

/// `Err` with the refusal for the first of `names` that is reserved. Empty names are skipped: a missing
/// name is another check's business.
pub fn check_all(names: &[&str]) -> Result<(), String> {
    for name in names {
        if let Some(word) = check(name) {
            return Err(refusal(name, word));
        }
    }
    Ok(())
}

/// The names among `names` that read as reserved, for listing the agents that already have one.
pub fn reserved_in<'a>(names: impl IntoIterator<Item = &'a str>) -> Vec<&'a str> {
    names.into_iter().filter(|n| check(n).is_some()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_reserved_word_is_refused_in_any_case_and_after_trimming() {
        for word in RESERVED {
            assert_eq!(check(word), Some(*word), "{word}");
            assert_eq!(check(&format!("  {}  ", word.to_uppercase())), Some(*word), "{word}");
        }
        // The hyphenated forms AgentMux itself uses.
        assert_eq!(check("github-consumer"), Some("githubconsumer"));
        assert_eq!(check("Swarm Forward"), Some("swarmforward"));
        assert_eq!(check("AgentMux"), Some("agentmux"));
    }

    #[test]
    fn look_alike_characters_and_separators_do_not_get_a_name_past() {
        for name in ["operator-3", "admin2", "user_10", "Op3rator", "0perator", "оperator", "ｏｐｅｒａｔｏｒ", "O.P.E.R.A.T.O.R", "oper\u{200B}ator", "sys_tem", "@dmin", "us3r", "hum@n", "agent-mux", "ΑgentMux"] {
            assert!(check(name).is_some(), "{name:?} slipped through");
        }
    }

    #[test]
    fn a_nickname_is_accepted() {
        for name in ["Wren", "Winny", "Pulp", "Clamk", "Loap-3", "Naki #2", "AgentY", "AgentX", "Opaz", "QTest", "Hostess", "Userland", "Administrate", "wren2", ""] {
            assert_eq!(check(name), None, "{name:?} refused");
        }
    }

    #[test]
    fn the_refusal_says_the_name_is_reserved_and_to_pick_a_nickname() {
        let e = check_all(&["fine", "Operator"]).unwrap_err();
        assert!(e.contains("reserved") && e.contains("nickname") && e.contains("\"Operator\""), "{e}");
        assert!(check_all(&["Wren", "wren"]).is_ok());
    }

    #[test]
    fn the_agents_that_already_have_a_reserved_name_are_listed() {
        assert_eq!(reserved_in(["wren", "Admin", "agentmux", "pulp"]), vec!["Admin", "agentmux"]);
    }
}

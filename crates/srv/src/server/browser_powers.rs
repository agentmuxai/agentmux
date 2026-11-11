// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What a browser-pane Auto-approve grant carries
//! (docs/specs/SPEC_BROWSER_WEB_TERMINAL_TYPING_2026_10_23.md §3).
//!
//! Every grant lifts the wait for committing clicks. On top of that the person
//! can add powers the grant names one by one, so a banner or a Settings row
//! says in plain words what an agent will be able to do on a site, and an
//! agent asking for a grant can't get more than the person was shown.

/// One thing a grant lets the agent do on the granted sites.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Power {
    /// Committing clicks go through without asking (what Auto-approve always did).
    Clicks,
    /// Type and run commands in a web terminal, no question per line.
    Terminal,
    /// Type in a web terminal, but the person is shown each line and presses
    /// Run it before Enter goes in (spec §4.4). Stricter than [`Power::Terminal`]:
    /// a grant that carries both asks.
    TerminalAsk,
    /// Press Enter and Space on the page, each taken as the click it is (spec §5):
    /// a form's default button, a button, a link or a checkbox, through the same
    /// check as a click. A key whose click can't be named is refused.
    EnterSpace,
    /// Fill the file chooser the page opened from a file in the agent's own workspace
    /// (spec §6): `set_files` with `chooser: true`. Without it the chooser is cancelled
    /// the moment the page opens it, as in any pane an agent drives.
    Uploads,
}

impl Power {
    /// The order a grant lists its powers in.
    const ALL: [Power; 5] = [Power::Clicks, Power::Terminal, Power::TerminalAsk, Power::EnterSpace, Power::Uploads];

    /// The name on the wire, in `powers` lists.
    pub(crate) fn name(self) -> &'static str {
        match self {
            Power::Clicks => "clicks",
            Power::Terminal => "terminal",
            Power::TerminalAsk => "terminal_ask",
            Power::EnterSpace => "enter_space",
            Power::Uploads => "uploads",
        }
    }

    fn from_name(name: &str) -> Option<Power> {
        Power::ALL
            .into_iter()
            .find(|p| p.name().eq_ignore_ascii_case(name.trim()))
    }
}

/// The powers an agent asked for, checked. Committing clicks are always in
/// (it is what a grant is); none asked means just that. An unknown name is an
/// error rather than ignored, so an agent doesn't think it asked for more than
/// the person will be shown.
pub(crate) fn parse(asked: Option<&[String]>) -> Result<Vec<Power>, String> {
    let mut out = vec![Power::Clicks];
    for name in asked.unwrap_or_default() {
        let p = Power::from_name(name).ok_or_else(|| {
            let known: Vec<&str> = Power::ALL.iter().map(|p| p.name()).collect();
            format!(
                "unknown power {name:?} in `powers`: ask for any of {}",
                known.join(", ")
            )
        })?;
        if !out.contains(&p) {
            out.push(p);
        }
    }
    out.sort_by_key(|p| Power::ALL.iter().position(|q| q == p));
    Ok(out)
}

/// `powers` with the terminal asking about each line: the person's "Allow, asking
/// for each line" on a request for the terminal.
pub(crate) fn asking_each_line(powers: &[Power]) -> Vec<Power> {
    let mut out: Vec<Power> = powers.iter().map(|p| if *p == Power::Terminal { Power::TerminalAsk } else { *p }).collect();
    out.dedup();
    out
}

/// The powers a standing grant can carry beyond committing clicks, which every
/// grant has: what Settings offers a checkbox for.
pub(crate) fn granted_by_the_person() -> Vec<String> {
    Power::ALL.iter().filter(|p| **p != Power::Clicks).map(|p| p.name().to_string()).collect()
}

/// The wire names of `powers`, for the pane's strip and the banner.
pub(crate) fn names(powers: &[Power]) -> Vec<&'static str> {
    powers.iter().map(|p| p.name()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asked(names: &[&str]) -> Vec<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn a_grant_always_has_clicks() {
        assert_eq!(parse(None).unwrap(), vec![Power::Clicks]);
        assert_eq!(parse(Some(&[])).unwrap(), vec![Power::Clicks]);
    }

    #[test]
    fn powers_are_listed_once_in_a_fixed_order() {
        let p = parse(Some(&asked(&["terminal", "Clicks", "terminal"]))).unwrap();
        assert_eq!(p, vec![Power::Clicks, Power::Terminal]);
        assert_eq!(names(&p), vec!["clicks", "terminal"]);
    }

    #[test]
    fn enter_and_space_is_its_own_power_listed_last() {
        let p = parse(Some(&asked(&["enter_space", "terminal"]))).unwrap();
        assert_eq!(p, vec![Power::Clicks, Power::Terminal, Power::EnterSpace]);
        assert_eq!(names(&p), vec!["clicks", "terminal", "enter_space"]);
        assert_eq!(asking_each_line(&p), vec![Power::Clicks, Power::TerminalAsk, Power::EnterSpace]);
        assert!(granted_by_the_person().contains(&"enter_space".to_string()));
    }

    #[test]
    fn uploads_is_its_own_power_listed_after_the_keys() {
        let p = parse(Some(&asked(&["uploads", "enter_space"]))).unwrap();
        assert_eq!(p, vec![Power::Clicks, Power::EnterSpace, Power::Uploads]);
        assert_eq!(names(&p), vec!["clicks", "enter_space", "uploads"]);
        assert!(granted_by_the_person().contains(&"uploads".to_string()));
    }

    #[test]
    fn allowing_with_a_question_per_line_swaps_the_terminal_for_its_stricter_form() {
        let p = parse(Some(&asked(&["terminal"]))).unwrap();
        assert_eq!(asking_each_line(&p), vec![Power::Clicks, Power::TerminalAsk]);
        assert_eq!(asking_each_line(&[Power::Clicks]), vec![Power::Clicks]);
    }

    #[test]
    fn an_unknown_power_is_an_error_naming_the_known_ones() {
        let e = parse(Some(&asked(&["terminal", "root"]))).unwrap_err();
        assert!(
            e.contains("\"root\"") && e.contains("clicks, terminal"),
            "{e}"
        );
    }
}

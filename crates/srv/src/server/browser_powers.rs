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
    /// Type and run commands in a web terminal.
    Terminal,
}

impl Power {
    /// The order a grant lists its powers in.
    const ALL: [Power; 2] = [Power::Clicks, Power::Terminal];

    /// The name on the wire, in `powers` lists.
    pub(crate) fn name(self) -> &'static str {
        match self {
            Power::Clicks => "clicks",
            Power::Terminal => "terminal",
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
    fn an_unknown_power_is_an_error_naming_the_known_ones() {
        let e = parse(Some(&asked(&["terminal", "root"]))).unwrap_err();
        assert!(
            e.contains("\"root\"") && e.contains("clicks, terminal"),
            "{e}"
        );
    }
}

// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Which Google services AgentMux's own client may ask for in this build
//! (SPEC_GOOGLE_PHASE_B_AGENTMUXS_OWN_CLIENT_2026_11_01.md D6, §8).
//!
//! Google reviews each tier of scopes per app, and an unreviewed one shows the person an unverified-app
//! screen and counts toward a 100-user cap. A public build therefore asks only for a service whose tier
//! Google has approved. The list is data: one edit in [`APPROVED`] when a tier is approved, no schema change.
//! An internal build (the people who run `agentmux-dev`, up to 100 users) turns every service on through the
//! same environment its client secret comes from.
//!
//! A client the person brought (BYO, the organization's internal one) is not limited: Google reviews it
//! against their own project, not ours.

/// Every service a Google account can be connected for, as the grant calls them.
pub const SERVICES: [&str; 3] = ["calendar", "drive", "gmail"];

/// The services whose tier Google has approved for AgentMux's own client, and which a public build offers.
/// Stage 1 adds "drive", stage 2 "calendar", stage 3 "gmail" (§8). None is approved yet.
const APPROVED: &[&str] = &[];

/// The environment variable that names the offered services, comma separated, in a developer's build.
pub const SERVICES_VAR: &str = "AGENTMUX_GOOGLE_SERVICES";

/// The services AgentMux's own Google client offers in this build.
pub fn offered() -> Vec<String> {
    let secret_given = std::env::var("AGENTMUX_GOOGLE_CLIENT_SECRET").is_ok_and(|v| !v.trim().is_empty());
    offered_with(std::env::var(SERVICES_VAR).ok().as_deref(), secret_given)
}

/// [`offered`] with the environment given: the list a developer named, else every service for a build run with
/// its client secret in the environment (an internal one), else the approved ones.
fn offered_with(named: Option<&str>, secret_in_environment: bool) -> Vec<String> {
    if let Some(list) = named.filter(|l| !l.trim().is_empty()) {
        return list
            .split(',')
            .map(|s| s.trim().to_ascii_lowercase())
            .filter(|s| SERVICES.contains(&s.as_str()))
            .collect();
    }
    let all: &[&str] = if secret_in_environment { &SERVICES } else { APPROVED };
    all.iter().map(|s| s.to_string()).collect()
}

/// The service a Google scope belongs to, or `None` for the sign-in scopes (`openid`, `email`, `profile`).
pub fn service_of_scope(scope: &str) -> Option<&'static str> {
    const API: &str = "https://www.googleapis.com/auth/";
    if scope == "https://mail.google.com/" || scope.strip_prefix(API).is_some_and(|s| s.starts_with("gmail.")) {
        return Some("gmail");
    }
    let name = scope.strip_prefix(API)?;
    if name == "calendar" || name.starts_with("calendar.") {
        Some("calendar")
    } else if name == "drive" || name.starts_with("drive.") {
        Some("drive")
    } else {
        None
    }
}

/// The sentence for the first of `requested` whose service is not in `offered`, if any.
pub fn first_not_offered(requested: &[String], offered: &[String]) -> Option<String> {
    let service = requested.iter().filter_map(|s| service_of_scope(s)).find(|s| !offered.iter().any(|o| o == s))?;
    let label = match service {
        "calendar" => "Calendar",
        "drive" => "Drive",
        _ => "Gmail",
    };
    Some(format!("{label} isn't available in this version of AgentMux yet."))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scopes(names: &[&str]) -> Vec<String> {
        names.iter().map(|n| format!("https://www.googleapis.com/auth/{n}")).collect()
    }

    #[test]
    fn a_public_build_offers_what_google_approved_and_an_internal_one_everything() {
        assert_eq!(offered_with(None, false), APPROVED.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        assert_eq!(offered_with(None, true), vec!["calendar", "drive", "gmail"]);
        assert_eq!(offered_with(Some("  "), true), vec!["calendar", "drive", "gmail"], "a blank list names nothing");
    }

    #[test]
    fn a_named_list_wins_and_ignores_what_it_does_not_know() {
        assert_eq!(offered_with(Some("Drive, calendar"), false), vec!["drive", "calendar"]);
        assert_eq!(offered_with(Some("drive,photos,"), true), vec!["drive"]);
    }

    #[test]
    fn each_scope_belongs_to_its_service_and_the_sign_in_scopes_to_none() {
        assert_eq!(service_of_scope("https://www.googleapis.com/auth/calendar.events.readonly"), Some("calendar"));
        assert_eq!(service_of_scope("https://www.googleapis.com/auth/calendar"), Some("calendar"));
        assert_eq!(service_of_scope("https://www.googleapis.com/auth/drive.file"), Some("drive"));
        assert_eq!(service_of_scope("https://www.googleapis.com/auth/gmail.compose"), Some("gmail"));
        assert_eq!(service_of_scope("https://mail.google.com/"), Some("gmail"));
        for sign_in in ["openid", "email", "profile", "https://www.googleapis.com/auth/userinfo.email"] {
            assert_eq!(service_of_scope(sign_in), None, "{sign_in}");
        }
    }

    #[test]
    fn a_request_stops_at_the_first_service_that_is_not_offered() {
        let offered = vec!["drive".to_string()];
        assert_eq!(first_not_offered(&scopes(&["drive.file"]), &offered), None);
        assert_eq!(first_not_offered(&["openid".to_string(), "email".to_string()], &[]), None, "sign-in needs no service");
        let why = first_not_offered(&scopes(&["drive.file", "gmail.readonly"]), &offered).unwrap();
        assert_eq!(why, "Gmail isn't available in this version of AgentMux yet.");
        let why = first_not_offered(&scopes(&["calendar.events.readonly"]), &offered).unwrap();
        assert!(why.starts_with("Calendar"), "{why}");
        assert!(first_not_offered(&scopes(&["gmail.readonly"]), &[]).is_some());
    }
}

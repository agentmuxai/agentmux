// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The check of a CI secret: a repository's or an environment's Actions secret
//! (docs/specs/SPEC_VAULT_LINKED_ENTRIES_FLEET_VAULT_AND_BATCH_IMPORT_2026_11_15.md §2.6).
//!
//! GitHub never gives a secret's value back. It gives a listing entry: the name and when it was last updated. This
//! reads that one entry with the GitHub CLI signed in on this computer, and keeps of the answer only whether the
//! secret is there and the day it last changed. The read is built from the entry's own link and nothing else: one
//! secret, a GET, a fixed endpoint. No other login is ever tried in the CLI's place.

use super::aws_import::Described;
use super::source_link::{Link, GITHUB_SECRET};
use crate::identity::aws_broker::mint::{RunError, RunSpec, Runner, RECIPE_TIMEOUT};

/// What the check says of a secret that isn't there, in a repository this login can see.
pub const NO_SUCH_SECRET: &str = "GitHub has no secret by that name there";
/// What it says when neither the secret nor its repository answers.
pub const CANT_SEE: &str = "the repository can't be seen with this computer's GitHub login";
const NO_CLI: &str = "this computer has no GitHub CLI (gh), which the check of a CI secret reads with";
const HOST: &str = "github.com";
/// What the desktop's keyring is reached through, where the CLI keeps its login on Linux. Passed on to the child
/// beside the allowlisted environment; neither is a credential.
const SESSION: [&str; 2] = ["DBUS_SESSION_BUS_ADDRESS", "XDG_RUNTIME_DIR"];

/// A CI secret's place, each part held to a strict name: they go into the path of a request.
#[derive(Debug, Clone, PartialEq)]
pub struct Place {
    pub owner: String,
    pub repo: String,
    pub environment: Option<String>,
    pub name: String,
}

/// Letters, digits and `extra`, not starting with a dash, and not a path's `.` or `..`.
fn named(s: &str, extra: &str, max: usize) -> bool {
    !s.is_empty() && s.len() <= max && !s.starts_with('-') && s != "." && s != ".." && s.chars().all(|c| c.is_ascii_alphanumeric() || extra.contains(c))
}

impl Place {
    /// The place `link` names, when every part of it is a name GitHub could have given. The Vault holds a link to
    /// the same when it is saved; it is held to it again here, where it becomes a request.
    pub fn of(link: &Link) -> Result<Place, String> {
        let wrong = |what: &str| format!("the entry's place is not a CI secret a check can read: {what}");
        if link.kind != GITHUB_SECRET {
            return Err(wrong("it is not of the kind github-secret"));
        }
        let (repository, environment) = link.location.split_once('@').map_or((link.location.as_str(), None), |(r, e)| (r, Some(e)));
        let Some((owner, repo)) = repository.split_once('/') else { return Err(wrong("the repository is owner/repo")) };
        if !named(owner, "-", 39) || !named(repo, "-_.", 100) {
            return Err(wrong("the repository is owner/repo, in letters, digits, - _ and ."));
        }
        if environment.is_some_and(|e| !named(e, "-_.", 100)) {
            return Err(wrong("the environment's name is letters, digits, - _ and ."));
        }
        if !named(&link.key, "_", 100) {
            return Err(wrong("the secret's name is letters, digits and _"));
        }
        Ok(Place { owner: owner.into(), repo: repo.into(), environment: environment.map(str::to_string), name: link.key.clone() })
    }

    /// `owner/repo`, as the Audit names the repository of a read.
    pub fn repository(&self) -> String {
        format!("{}/{}", self.owner, self.repo)
    }

    fn repository_endpoint(&self) -> String {
        format!("repos/{}/{}", self.owner, self.repo)
    }

    /// The one listing entry: the repository's secret, or the environment's.
    fn secret_endpoint(&self) -> String {
        match &self.environment {
            None => format!("{}/actions/secrets/{}", self.repository_endpoint(), self.name),
            Some(environment) => format!("{}/environments/{environment}/secrets/{}", self.repository_endpoint(), self.name),
        }
    }
}

/// The arguments of the one kind of run there is: a GET of `endpoint` at github.com, with the answer's status line.
fn argv(endpoint: String) -> Vec<String> {
    ["api", "--hostname", HOST, "--method", "GET", "--include"].iter().map(|s| s.to_string()).chain([endpoint]).collect()
}

/// The status code and the body of an answer printed with its status line and headers.
fn answer(stdout: &[u8]) -> Option<(u16, String)> {
    let text = String::from_utf8_lossy(stdout);
    let status = text.lines().next()?;
    let code = status.strip_prefix("HTTP/")?.split_whitespace().nth(1)?.parse().ok()?;
    let body = text.split_once("\r\n\r\n").or_else(|| text.split_once("\n\n")).map_or("", |(_, body)| body);
    Some((code, body.to_string()))
}

/// Runs the GitHub CLI for `endpoint` and gives GitHub's answer: its status code and body. The CLI's own words are
/// read only to say why there was no answer, and none of them is kept.
async fn ask<R: Runner>(runner: &R, endpoint: String) -> Result<(u16, String), String> {
    let Some(program) = runner.which("gh") else { return Err(NO_CLI.into()) };
    let mut env: Vec<(String, String)> = [("GH_PROMPT_DISABLED", "1"), ("GH_NO_UPDATE_NOTIFIER", "1"), ("NO_COLOR", "1")].iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
    env.extend(SESSION.iter().filter_map(|name| Some((name.to_string(), std::env::var(name).ok()?))));
    let out = match runner.run(RunSpec { program, args: argv(endpoint), env, timeout: RECIPE_TIMEOUT }).await {
        Ok(out) => out,
        Err(RunError::TimedOut) => return Err("GitHub took too long to answer (25 s). Try again in a minute.".into()),
        Err(RunError::Spawn(_)) => return Err("couldn't run the GitHub CLI on this computer".into()),
    };
    match answer(&out.stdout) {
        Some(said) => Ok(said),
        None if out.stderr.contains("gh auth login") => Err("the GitHub CLI on this computer is signed out: a person runs `gh auth login` there".into()),
        None => Err("the GitHub CLI got no answer from GitHub: `gh auth status` on this computer says whether it is signed in".into()),
    }
}

/// Reads the listing entry of the secret `link` names. A secret that isn't there, in a repository this login can
/// see, fails with [`NO_SUCH_SECRET`]; the two "not found" answers are told apart by asking for the repository, and
/// by each answer's status code, never by its text. Of the entry only the day it was last updated is kept.
pub async fn describe<R: Runner>(runner: &R, link: &Link) -> Result<Described, String> {
    let place = Place::of(link)?;
    match ask(runner, place.secret_endpoint()).await? {
        (200, body) => {
            let said: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();
            if !said["name"].as_str().is_some_and(|name| name.eq_ignore_ascii_case(&place.name)) {
                return Err("GitHub answered with something that isn't that secret's listing entry".into());
            }
            let changed = said["updated_at"].as_str().and_then(|at| chrono::DateTime::parse_from_rfc3339(at.trim()).ok()).map(|t| t.with_timezone(&chrono::Utc).date_naive());
            Ok(Described { changed, rotation: None, deleted: false })
        }
        (404, _) => match ask(runner, place.repository_endpoint()).await? {
            (200, _) => Err(NO_SUCH_SECRET.into()),
            _ => Err(CANT_SEE.into()),
        },
        (401, _) => Err("GitHub refused this computer's GitHub login: a person signs in again with `gh auth login`".into()),
        (403, _) => Err("GitHub refused the read (status 403): this computer's GitHub login may not read that repository's secrets".into()),
        (code, _) => Err(format!("GitHub answered with status {code}")),
    }
}

#[cfg(test)]
#[path = "github_secret_tests.rs"]
pub(super) mod tests;

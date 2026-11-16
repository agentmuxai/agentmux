// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The read of a CI secret's listing entry: what each answer of GitHub comes to, that the run is the fixed one for
//! the link, and that a link which is not a strict name runs nothing. Every repository and secret here is invented.

use std::path::PathBuf;
use std::sync::Mutex;

use super::*;
use crate::identity::aws_broker::mint::RunOutput;

const SECRET: &str = "repos/example-org/example-app/actions/secrets/EXAMPLE_TOKEN";
const COLLECTION: &str = "repos/example-org/example-app/actions/secrets?per_page=1";
/// What the collection answers: one entry, of a secret nobody asked about.
const NEIGHBOUR: &str = r#"{"total_count": 3, "secrets": [{"name": "NEIGHBOUR_SENTINEL_2291", "updated_at": "2026-02-02T00:00:00Z"}]}"#;
const ENTRY: &str = r#"{"name": "EXAMPLE_TOKEN", "created_at": "2026-01-05T10:00:00Z", "updated_at": "2026-10-01T23:30:00Z"}"#;

/// A stand-in GitHub CLI: each endpoint it knows answers with a status and a body, as `--include` prints them.
pub(crate) struct Gh {
    pub known: Vec<(&'static str, u16, &'static str)>,
    /// What the CLI does when it gives no answer of GitHub's: its stderr, or a run that never ends.
    pub silent: Option<&'static str>,
    pub installed: bool,
    pub runs: Mutex<Vec<RunSpec>>,
}

impl Gh {
    pub fn with(known: &[(&'static str, u16, &'static str)]) -> Self {
        Self { known: known.to_vec(), silent: None, installed: true, runs: Mutex::new(Vec::new()) }
    }

    /// The endpoint of every run, in order.
    pub fn asked(&self) -> Vec<String> {
        self.runs.lock().unwrap().iter().map(|run| run.args.last().cloned().unwrap_or_default()).collect()
    }
}

impl Runner for Gh {
    fn which(&self, program: &str) -> Option<PathBuf> {
        (self.installed && program == "gh").then(|| PathBuf::from("/bin/gh"))
    }

    async fn run(&self, spec: RunSpec) -> Result<RunOutput, RunError> {
        let endpoint = spec.args.last().cloned().unwrap_or_default();
        self.runs.lock().unwrap().push(spec);
        match (self.silent, self.known.iter().find(|(known, _, _)| *known == endpoint)) {
            (Some("never"), _) => Err(RunError::TimedOut),
            (Some(said), _) => Ok(RunOutput { success: false, stdout: Vec::new(), stderr: said.into() }),
            (None, Some((_, status, body))) => {
                let printed = format!("HTTP/2.0 {status} Whatever\r\nContent-Type: application/json\r\nX-Made-Up: 1\r\n\r\n{body}");
                Ok(RunOutput { success: *status == 200, stdout: printed.into_bytes(), stderr: String::new() })
            }
            (None, None) => Ok(RunOutput { success: false, stdout: b"HTTP/2.0 404 Not Found\r\n\r\n{\"message\": \"Not Found\"}".to_vec(), stderr: "gh: Not Found (HTTP 404)".into() }),
        }
    }
}

fn link(location: &str, key: &str) -> Link {
    Link { kind: GITHUB_SECRET.into(), location: location.into(), key: key.into(), ..Default::default() }
}

fn place() -> Link {
    link("example-org/example-app", "EXAMPLE_TOKEN")
}

fn described(gh: &Gh, link: &Link) -> Result<Described, String> {
    tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(describe(gh, link))
}

#[test]
fn a_secret_that_is_there_gives_the_day_it_last_changed_and_nothing_else() {
    let gh = Gh::with(&[(SECRET, 200, ENTRY)]);
    let found = described(&gh, &place()).unwrap();
    // The day in UTC, no rotation setting, and nothing of the rest of the entry.
    assert_eq!(found, Described { changed: chrono::NaiveDate::from_ymd_opt(2026, 10, 1), rotation: None, deleted: false });
    assert_eq!(gh.asked(), [SECRET]);
}

#[test]
fn the_run_is_one_get_of_the_fixed_endpoint_with_no_login_of_its_own() {
    let gh = Gh::with(&[(SECRET, 200, ENTRY)]);
    described(&gh, &place()).unwrap();
    let environment = "repos/example-org/example-app/environments/production/secrets/DEPLOY_KEY";
    described(&gh, &link("example-org/example-app@production", "DEPLOY_KEY")).unwrap_err();
    let runs = gh.runs.lock().unwrap();
    let args = |i: usize| runs[i].args.iter().map(String::as_str).collect::<Vec<_>>();
    assert_eq!(args(0), ["api", "--hostname", "github.com", "--method", "GET", "--include", SECRET]);
    assert_eq!(args(1), ["api", "--hostname", "github.com", "--method", "GET", "--include", environment]);
    // After a "not found" the collection the secret is in is asked for one entry, the same way.
    assert_eq!(args(2), ["api", "--hostname", "github.com", "--method", "GET", "--include", "repos/example-org/example-app/environments/production/secrets?per_page=1"]);
    assert_eq!(runs.len(), 3);
    for run in runs.iter() {
        assert_eq!(run.program, PathBuf::from("/bin/gh"));
        assert!(run.env.iter().all(|(name, _)| !name.contains("TOKEN")), "no login is handed to the CLI: {:?}", run.env);
    }
}

#[test]
fn a_secret_that_is_not_there_is_told_from_secrets_this_login_cant_read_by_status() {
    // This login can read the secrets there, and the one asked for is not among them.
    let gone = Gh::with(&[(COLLECTION, 200, NEIGHBOUR)]);
    let said = described(&gone, &place()).unwrap_err();
    assert_eq!(said, NO_SUCH_SECRET);
    assert_eq!(gone.asked(), [SECRET, COLLECTION]);
    // The collection's answer is dropped unread: a name in it is in nothing that comes back.
    assert!(!said.contains("NEIGHBOUR"));
    // A repository this login can see is not enough: the secrets there must be readable, or nothing is known.
    for unreadable in [Gh::with(&[]), Gh::with(&[(COLLECTION, 403, NEIGHBOUR)]), Gh::with(&[("repos/example-org/example-app", 200, "{}")])] {
        assert_eq!(described(&unreadable, &place()).unwrap_err(), CANT_READ);
        assert_eq!(unreadable.asked(), [SECRET, COLLECTION], "the repository itself is never asked for");
    }
    // The status is the first line's, and nothing else of what was printed is looked at for it.
    assert_eq!(Answer(NEIGHBOUR.as_bytes().to_vec()).status(), None);
    assert_eq!(Answer(b"HTTP/2.0 200 OK\r\n\r\n{}".to_vec()).status(), Some(200));
}

#[test]
fn a_refusal_or_no_answer_says_why_and_runs_nothing_else() {
    let said = |gh: &Gh| described(gh, &place()).unwrap_err();
    let not_allowed = Gh::with(&[(SECRET, 403, r#"{"message": "made-up refusal text"}"#)]);
    assert!(said(&not_allowed).contains("may not read that repository's secrets"));
    assert!(!said(&not_allowed).contains("made-up"), "nothing of GitHub's text is kept");
    assert!(said(&Gh::with(&[(SECRET, 401, "{}")])).contains("gh auth login"));
    assert_eq!(said(&Gh::with(&[(SECRET, 500, "{}")])), "GitHub answered with status 500");
    assert_eq!(not_allowed.asked(), [SECRET, SECRET], "one run a read, and no other endpoint");

    let signed_out = Gh { silent: Some("To get started with GitHub CLI, please run:  gh auth login"), ..Gh::with(&[]) };
    assert!(said(&signed_out).contains("is signed out"));
    let offline = Gh { silent: Some("error connecting to made-up.example"), ..Gh::with(&[]) };
    assert!(said(&offline).contains("got no answer from GitHub"));
    assert!(!said(&offline).contains("made-up"));
    let slow = Gh { silent: Some("never"), ..Gh::with(&[]) };
    assert!(said(&slow).contains("took too long"));

    let none = Gh { installed: false, ..Gh::with(&[(SECRET, 200, ENTRY)]) };
    assert!(said(&none).contains("no GitHub CLI"));
    assert!(none.asked().is_empty(), "no other program is run in its place");
}

#[test]
fn an_answer_that_is_not_that_secrets_entry_is_not_taken_for_it() {
    let other = Gh::with(&[(SECRET, 200, r#"{"name": "ANOTHER", "updated_at": "2026-10-01T00:00:00Z"}"#)]);
    assert!(described(&other, &place()).unwrap_err().contains("isn't that secret's listing entry"));
    // GitHub writes names in capitals; an entry with no date is still there.
    let no_date = Gh::with(&[(SECRET, 200, r#"{"name": "example_token"}"#)]);
    assert_eq!(described(&no_date, &place()).unwrap(), Described::default());
}

#[test]
fn a_link_that_is_not_a_strict_name_runs_nothing() {
    let gh = Gh::with(&[(SECRET, 200, ENTRY)]);
    for (location, key) in [
        ("-owner/example-app", "EXAMPLE_TOKEN"),
        ("example-org/-app", "EXAMPLE_TOKEN"),
        ("example-org/example-app@-production", "EXAMPLE_TOKEN"),
        ("example-org/example-app", "-TOKEN"),
        ("example-org/..", "EXAMPLE_TOKEN"),
        ("example-org/example-app@..", "EXAMPLE_TOKEN"),
        ("example-org/example-app/actions", "EXAMPLE_TOKEN"),
        ("example-org", "EXAMPLE_TOKEN"),
        ("example-org/example app", "EXAMPLE_TOKEN"),
        ("example-org/example-app?x=1", "EXAMPLE_TOKEN"),
        ("example.org/example-app", "EXAMPLE_TOKEN"),
        ("example-org/example-app", "EXAMPLE/TOKEN"),
        ("example-org/example-app", "{owner}"),
        ("example-org/example-app", ""),
    ] {
        let why = described(&gh, &link(location, key)).unwrap_err();
        assert!(why.contains("not a CI secret a check can read"), "{location} {key}: {why}");
    }
    let aws = Link { kind: "aws-secret".into(), ..place() };
    assert!(described(&gh, &aws).is_err());
    assert!(gh.asked().is_empty());
    assert_eq!(Place::of(&link("example-org/example-app@staging-2", "A_1")).unwrap().repository(), "example-org/example-app");
}

// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A linked entry: what one may hold, that an agent is refused a value it does not have, what `list`, `show`, `health`
//! and `verify` say of it, and that its check reads a place's description and never the key
//! (docs/specs/SPEC_VAULT_LINKED_ENTRIES_FLEET_VAULT_AND_BATCH_IMPORT_2026_11_15.md §2). Every place named here is
//! invented.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use super::*;
use crate::backend::storage::vault::VaultActivityRow;
use crate::backend::vault::aws_import::{ImportRequest, Source};
use crate::backend::vault::github_secret::tests::Gh;
use crate::backend::vault::inspect::{self, UpdateRequest};
use crate::backend::vault::seal;
use crate::backend::vault::source_link::{plan_for_agent, plan_for_person, AgentLink, CHANGED, CHECKED, COULD_NOT_READ, KEY, KIND, LOCATION, MISSING, NOT_STARTED, PRESENT, PROFILE, REGION};
use crate::backend::vault::{command_target, create, fill_target, list_for_agent, update, EntryChange};
use crate::identity::aws_broker::mint::{RunError, RunOutput, RunSpec};

const VALUE: &str = "link-sentinel-7731-qd";
const REASON: &str = "see whether the invoice key changed before task 31007";
const READERS_TEXT: &str = "invoice-sync (lambda): at run time\nrelease (workflow): on a tag";
const COPIES_TEXT: &str = "github-secret: example-org/example-app#EXAMPLE_TOKEN\naws-secret: prod/invoice-sync-replica#api.token";
const NOT_FOUND: &str = "An error occurred (ResourceNotFoundException) when calling the DescribeSecret operation: Secrets Manager can't find the specified secret.";
const DENIED: &str = "An error occurred (AccessDeniedException) when calling the DescribeSecret operation: User is not authorized to perform that action";

fn store() -> Arc<Store> {
    seal::use_test_key();
    let s = Store::open_in_memory().unwrap();
    s.apply_identity_schema_for_tests().unwrap();
    Arc::new(s)
}

fn fields(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
}

/// A link to an AWS secret's key, in a region.
fn place() -> Vec<(&'static str, &'static str)> {
    vec![(LINK, ONLY), (KIND, AWS_SECRET), (LOCATION, "prod/invoice-sync"), (KEY, "api.token"), (REGION, "eu-west-1")]
}

fn entry(f: &[(&str, &str)], value: Option<&str>) -> NewEntry {
    let parts = value.map(|v| Parts([("value".to_string(), v.to_string())].into())).unwrap_or_default();
    NewEntry { kind: "api-key".into(), label: "Invoice provider key".into(), fields: fields(f), tags: vec![], parts }
}

fn linked_with(s: &Store, more: &[(&str, &str)]) -> String {
    create(s, entry(&[place(), more.to_vec()].concat(), None)).unwrap()
}

fn rows(s: &Store, action: &str) -> Vec<VaultActivityRow> {
    s.vault_activity_all().unwrap().into_iter().filter(|r| r.action == action).collect()
}

/// A stand-in AWS CLI: each secret it knows answers with its description, and any other with what AWS said. Beside
/// it a stand-in GitHub CLI, which knows no secret and no repository unless it is given some.
struct Cli {
    known: Vec<(&'static str, Result<&'static str, &'static str>)>,
    /// The operation and the secret of every run.
    seen: Mutex<Vec<(String, String)>>,
    gh: Gh,
}

impl Cli {
    fn with(known: &[(&'static str, Result<&'static str, &'static str>)]) -> Self {
        Self { known: known.to_vec(), seen: Mutex::new(Vec::new()), gh: Gh::with(&[]) }
    }

    /// With what GitHub answers for each endpoint: a status and a body.
    fn github(self, known: &[(&'static str, u16, &'static str)]) -> Self {
        Self { gh: Gh::with(known), ..self }
    }

    fn seen(&self) -> Vec<(String, String)> {
        self.seen.lock().unwrap().clone()
    }
}

impl Runner for Cli {
    fn which(&self, program: &str) -> Option<PathBuf> {
        if program == "gh" {
            return self.gh.which(program);
        }
        Some(PathBuf::from("/bin/aws"))
    }

    async fn run(&self, spec: RunSpec) -> Result<RunOutput, RunError> {
        if spec.program == std::path::Path::new("/bin/gh") {
            return self.gh.run(spec).await;
        }
        let arg = |i: usize| spec.args.get(i).cloned().unwrap_or_default();
        let (operation, id) = (arg(1), arg(3));
        self.seen.lock().unwrap().push((operation, id.clone()));
        Ok(match self.known.iter().find(|(name, _)| *name == id).map(|(_, said)| *said) {
            Some(Ok(description)) => RunOutput { success: true, stdout: description.as_bytes().to_vec(), stderr: String::new() },
            Some(Err(said)) => RunOutput { success: false, stdout: Vec::new(), stderr: said.into() },
            None => RunOutput { success: false, stdout: Vec::new(), stderr: NOT_FOUND.into() },
        })
    }
}

const UNCHANGED: &str = r#"{"changed": "2026-09-30T23:10:00.120000+00:00", "rotation": true, "deleted": null}"#;
const CHANGED_LATER: &str = r#"{"changed": "2026-11-02T08:00:00+00:00", "rotation": false, "deleted": null}"#;
const BEING_DELETED: &str = r#"{"changed": 1790000000.5, "rotation": null, "deleted": "2026-11-20T00:00:00+00:00"}"#;

#[test]
fn a_linked_entry_names_its_place_and_holds_no_value() {
    let s = store();
    let id = linked_with(&s, &[(READERS, READERS_TEXT), (ROTATED, "2026-10-01"), (COPIES, COPIES_TEXT), ("expires", "2027-03-01")]);
    let row = s.vault_get(&id).unwrap().unwrap();
    assert!(is_linked_row(&row));
    assert!(seal::open(&seal::key(false).unwrap(), &row.id, &row.sealed).unwrap().0.is_empty());
    assert_eq!(copies(&fields(&[(COPIES, COPIES_TEXT)])).iter().map(place_text).collect::<Vec<_>>(), ["github-secret: example-org/example-app#EXAMPLE_TOKEN", "aws-secret: prod/invoice-sync-replica#api.token"]);

    for (bad, value, says) in [
        (vec![(LINK, ONLY)], None, "says where the key lives"),
        (place(), Some(VALUE), "holds no value"),
        (vec![(LINK, "yes"), (KIND, AWS_SECRET), (LOCATION, "prod/invoice-sync")], None, "link is \"only\""),
        ([place(), vec![(ROTATED, "last spring")]].concat(), None, "rotated is the date"),
        ([place(), vec![(READERS, "a\nb\nc\nd\ne\nf\ng\nh\ni")]].concat(), None, "at most 8 lines"),
        ([place(), vec![(COPIES, "other: a\nother: b\nother: c\nother: d\nother: e")]].concat(), None, "at most 4"),
        ([place(), vec![(COPIES, "ftp: files.example.com#key")]].concat(), None, "isn't a kind of machine copy"),
        ([place(), vec![(COPIES, "somewhere in the office")]].concat(), None, "it names no kind"),
        // A date of last change is a link's own: the Vault's history dates a value it holds.
        (vec![(ROTATED, "2026-10-01")], Some(VALUE), "rotated is for a linked entry"),
    ] {
        let said = create(&s, entry(&bad, value)).unwrap_err();
        assert!(said.contains(says), "{bad:?}: {said}");
        assert!(!said.contains(VALUE), "{said}");
    }
    // A saved sign-in is filled with its password, which a link does not have.
    let sign_in = NewEntry { kind: "login".into(), ..entry(&[place(), vec![("site", "https://example.test")]].concat(), None) };
    assert!(create(&s, sign_in).unwrap_err().contains("not a saved sign-in"));
    // Who reads a key and where else it is kept may be said of an entry that holds its value too.
    assert!(create(&s, NewEntry { label: "Deploy token".into(), ..entry(&[(READERS, "release (workflow): on a tag"), (COPIES, "other: the build server")], Some(VALUE)) }).is_ok());
}

#[test]
fn whether_an_entry_is_a_link_is_set_when_it_is_made() {
    let s = store();
    let link = linked_with(&s, &[]);
    let held = create(&s, NewEntry { label: "Deploy token".into(), ..entry(&[], Some(VALUE)) }).unwrap();
    let change = |id: &str, f: &[(&str, &str)], value: Option<&str>| {
        let label = s.vault_get(id).unwrap().unwrap().label;
        let parts = value.map(|v| Parts([("value".to_string(), v.to_string())].into())).unwrap_or_default();
        update(&s, EntryChange { id: id.into(), label, fields: fields(f), tags: vec![], parts, clear: vec![] })
    };
    assert_eq!(change(&held, &place(), None), Err(SET_WHEN_MADE.into()));
    assert_eq!(change(&link, &[(KIND, AWS_SECRET), (LOCATION, "prod/invoice-sync")], Some(VALUE)), Err(SET_WHEN_MADE.into()));
    assert!(change(&link, &place(), Some(VALUE)).unwrap_err().contains("holds no value"));
    // The person changes what a link says, and it stays a link.
    assert_eq!(change(&link, &[place(), vec![(READERS, "invoice-sync (lambda): at run time")]].concat(), None), Ok(()));
    assert!(is_linked_row(&s.vault_get(&link).unwrap().unwrap()));

    let asked = |id: &str, f: &[(&str, &str)]| UpdateRequest { id: id.into(), label: None, fields: fields(f), tags: None, reason: Some(REASON.into()) };
    assert_eq!(inspect::link_change(&s, &asked(&held, &place()), "builder"), Err(SET_WHEN_MADE.into()));
    assert_eq!(inspect::link_change(&s, &asked(&link, &[(LINK, "")]), "builder"), Err(SET_WHEN_MADE.into()));
}

#[test]
fn an_agent_is_refused_a_linked_entrys_value_in_one_sentence() {
    let s = store();
    let id = linked_with(&s, &[]);
    let sentence = "This entry holds no value: the key is kept at aws-secret: prod/invoice-sync#api.token. Ask the person, or use the access you already have there.";
    assert_eq!(fill_target(&s, &id, Some("builder")).err(), Some(sentence.to_string()));
    assert_eq!(command_target(&s, &id, Some("builder")).err(), Some(sentence.to_string()));
    assert_eq!(command_target(&s, &format!("{id}@previous"), Some("builder")).err(), Some(sentence.to_string()));
    // An import can't give it a value either: the value goes in as a new entry.
    let import = ImportRequest {
        source: Source { aws_secret_id: "prod/invoice-sync".into(), ..Default::default() },
        kind: String::new(),
        label: String::new(),
        fields: BTreeMap::new(),
        vault: None,
        id: Some(id.clone()),
        reason: Some(REASON.into()),
        refs: vec![],
    };
    assert_eq!(aws_import::check(&s, &import, "builder").unwrap_err(), "Invoice provider key is a linked entry, which holds no value: import the value as a new entry");
    // Nothing was asked of the person and nothing was recorded as a use.
    assert!(s.vault_activity_all().unwrap().iter().all(|r| r.action == "created"));
}

#[test]
fn list_and_show_tell_an_agent_the_place_the_readers_and_the_copies_and_never_the_login() {
    let s = store();
    let id = linked_with(&s, &[(PROFILE, "work-read"), (READERS, READERS_TEXT), (ROTATED, "2026-10-01"), (COPIES, COPIES_TEXT)]);
    let listed = list_for_agent(&s, Some("builder")).unwrap().remove(0);
    let expected = AgentLink {
        kind: AWS_SECRET.into(),
        location: "prod/invoice-sync".into(),
        key: "api.token".into(),
        checked: None,
        link_only: true,
        readers: lines(READERS_TEXT).into_iter().map(str::to_string).collect(),
        rotated: Some("2026-10-01".into()),
        copies: lines(COPIES_TEXT).into_iter().map(str::to_string).collect(),
    };
    assert_eq!(listed.source, Some(expected));
    let shown = inspect::show(&s, &id, Some("builder"), &seal::key(false), chrono::NaiveDate::from_ymd_opt(2026, 11, 15).unwrap()).unwrap();
    assert!(shown.link_only && shown.readable && shown.parts_set.is_empty() && shown.checked.is_none());
    assert_eq!(shown.fields.get(READERS).map(String::as_str), Some(READERS_TEXT));
    let told = format!("{} {}", serde_json::to_string(&listed).unwrap(), serde_json::to_string(&shown).unwrap());
    assert!(!told.contains("work-read") && !told.contains(PROFILE), "{told}");
}

#[tokio::test]
async fn a_check_reads_the_places_description_and_never_the_key() {
    let s = store();
    let id = linked_with(&s, &[(ROTATED, "2026-10-01"), (COPIES, COPIES_TEXT)]);
    let plan = plan_for_agent(&s, &id, "builder", Some(REASON)).unwrap();
    // Before any check the pane has both other places, neither checked.
    let before = copy_checks(&s, &s.vault_get(&id).unwrap().unwrap());
    assert_eq!((before.len(), before.iter().filter(|c| c.checked.is_some()).count()), (2, 0));
    let cli = Cli::with(&[("prod/invoice-sync", Ok(UNCHANGED)), ("prod/invoice-sync-replica", Ok(CHANGED_LATER))]).github(&[(CI_SECRET, 200, CI_ENTRY)]);
    let checked = source_link::check(&s, &cli, &plan, Via::Host).await;
    assert_eq!((checked.outcome, checked.why.as_str(), checked.source.as_str()), (PRESENT, "", "aws-secret:prod/invoice-sync#api.token"));
    assert_eq!((checked.last_changed.as_deref(), checked.rotation), (Some("2026-09-30"), Some(true)));
    // Each other place is in the answer; the one in AWS was described under the entry's own region, and the CI
    // secret's listing entry was read with the GitHub CLI.
    let copies: Vec<(&str, &str)> = checked.copies.iter().map(|c| (c.place.as_str(), c.outcome)).collect();
    assert_eq!(copies, [("github-secret: example-org/example-app#EXAMPLE_TOKEN", PRESENT), ("aws-secret: prod/invoice-sync-replica#api.token", CHANGED)]);
    assert_eq!(checked.copies[1].why, "the place last changed on 2026-11-02, after the 2026-10-01 recorded");
    let described: Vec<(String, String)> = [("describe-secret", "prod/invoice-sync"), ("describe-secret", "prod/invoice-sync-replica")].iter().map(|(o, i)| (o.to_string(), i.to_string())).collect();
    assert_eq!(cli.seen(), described);
    assert_eq!(cli.gh.asked(), [CI_SECRET]);

    // A line for the entry's own place and one for each copy that was asked, each naming its place.
    let own = rows(&s, CHECKED);
    assert_eq!(own.iter().map(|r| (r.target.as_str(), r.outcome.as_str(), r.actor.as_str(), r.reason.as_str())).collect::<Vec<_>>(), [("aws-secret:prod/invoice-sync#api.token", PRESENT, "builder", REASON)]);
    let other = rows(&s, CHECKED_COPY);
    assert_eq!(other.iter().map(|r| (r.target.as_str(), r.outcome.as_str())).collect::<Vec<_>>()[0], ("github-secret:example-org/example-app#EXAMPLE_TOKEN", PRESENT));
    assert_eq!(other[1].target, "aws-secret:prod/invoice-sync-replica#api.token");
    assert!(other[1].outcome.starts_with("changed since recorded: the place last changed on 2026-11-02"));
    // The newest check is the entry's own place's, whatever its copies' lines say.
    let row = s.vault_get(&id).unwrap().unwrap();
    assert_eq!(source_link::last_check(&s, &row, &Link::of_row(&row).unwrap()).map(|c| c.outcome), Some(PRESENT.to_string()));
    assert_eq!(standing(&s, &row), Some((Standing::Checked, false)));
    // What the pane shows of each other place: its newest check.
    let known = copy_checks(&s, &row);
    let said: Vec<(&str, &str, bool)> = known.iter().map(|c| (c.place.as_str(), c.cannot.as_str(), c.checked.is_some())).collect();
    assert_eq!(said, [("github-secret: example-org/example-app#EXAMPLE_TOKEN", "", true), ("aws-secret: prod/invoice-sync-replica#api.token", "", true)]);
    assert!(known[1].checked.as_ref().unwrap().outcome.starts_with("changed since recorded: the place last changed on 2026-11-02"));
    // A place a person described in words is the one kind no check describes, and the pane says so.
    let worded = create(&s, NewEntry { label: "Mail provider key".into(), ..entry(&[place(), vec![(COPIES, "other: the build host's config file")]].concat(), None) }).unwrap();
    assert_eq!(copy_checks(&s, &s.vault_get(&worded).unwrap().unwrap())[0].cannot, NOT_FROM_HERE);
}

#[tokio::test]
async fn a_place_that_changed_is_missing_or_refuses_is_what_the_check_says() {
    let s = store();
    let id = linked_with(&s, &[(ROTATED, "2026-10-01")]);
    let plan = plan_for_person(&s, &id).unwrap();
    let row = || s.vault_get(&id).unwrap().unwrap();
    for (said, outcome, why, stands) in [
        (Ok(CHANGED_LATER), CHANGED, "the place last changed on 2026-11-02, after the 2026-10-01 recorded", Standing::NeedsALook),
        (Err(NOT_FOUND), MISSING, "AWS has no secret by that id, in that region", Standing::NeedsALook),
        (Ok(BEING_DELETED), MISSING, "the secret is scheduled for deletion", Standing::NeedsALook),
        (Err(DENIED), COULD_NOT_READ, "AWS refused: this identity can't read that secret", Standing::Checked),
    ] {
        let checked = source_link::check(&s, &Cli::with(&[("prod/invoice-sync", said)]), &plan, Via::Host).await;
        assert_eq!((checked.outcome, checked.why.as_str()), (outcome, why));
        assert_eq!(standing(&s, &row()), Some((stands, false)), "{outcome}");
    }
    // A link with no date recorded is present, and the answer says when the place last changed.
    let undated = create(&s, NewEntry { label: "Mail provider key".into(), ..entry(&place(), None) }).unwrap();
    let checked = source_link::check(&s, &Cli::with(&[("prod/invoice-sync", Ok(CHANGED_LATER))]), &plan_for_person(&s, &undated).unwrap(), Via::Host).await;
    assert_eq!((checked.outcome, checked.last_changed.as_deref(), checked.rotation), (PRESENT, Some("2026-11-02"), Some(false)));

    // A copy in AWS under a place that isn't in AWS has no region or login to be described under: the check says
    // so and runs nothing for it.
    let ci = create(&s, NewEntry { label: "Store upload key".into(), ..entry(&[(LINK, ONLY), (KIND, source_link::GITHUB_SECRET), (LOCATION, "example-org/example-app"), (KEY, "EXAMPLE_TOKEN"), (COPIES, "aws-secret: prod/store-key")], None) }).unwrap();
    let cli = Cli::with(&[]).github(&[(CI_SECRET, 200, CI_ENTRY)]);
    let unread = source_link::check(&s, &cli, &plan_for_person(&s, &ci).unwrap(), Via::Host).await;
    assert_eq!((unread.outcome, unread.last_changed.as_deref(), cli.seen().len()), (PRESENT, Some("2026-10-01"), 0));
    assert_eq!((unread.copies[0].outcome, unread.copies[0].why.as_str()), (COULD_NOT_READ, NOT_UNDER_AWS));
    assert!(rows(&s, CHECKED_COPY).is_empty());
    // The pane says the same of that copy without a line in the history, and nothing of an entry that names none.
    let known = copy_checks(&s, &s.vault_get(&ci).unwrap().unwrap());
    assert_eq!(known, [VaultCopyCheck { place: "aws-secret: prod/store-key".into(), checked: None, cannot: NOT_UNDER_AWS.into() }]);
    assert!(copy_checks(&s, &row()).is_empty());
    // A link nobody has checked as it is now is unchecked.
    let never = create(&s, NewEntry { label: "Queue key".into(), ..entry(&place(), None) }).unwrap();
    assert_eq!(standing(&s, &s.vault_get(&never).unwrap().unwrap()), Some((Standing::Unchecked, true)));
    assert_eq!(standing(&s, &s.vault_get(&create(&s, NewEntry { label: "Deploy token".into(), ..entry(&[], Some(VALUE)) }).unwrap()).unwrap().unwrap()), None);
}

#[test]
fn an_old_check_is_unchecked_whatever_it_found() {
    let now = 1_800_000_000_000;
    let at = |days: i64, outcome: &str| VaultSourceCheck { at: now - days * DAY_MS, outcome: outcome.into() };
    assert_eq!(stands(Some(&at(1, PRESENT)), now), (Standing::Checked, false));
    assert_eq!(stands(Some(&at(UNCHECKED_DAYS, PRESENT)), now), (Standing::Checked, false));
    assert_eq!(stands(Some(&at(UNCHECKED_DAYS + 1, PRESENT)), now), (Standing::Unchecked, true));
    // A place found changed or missing long ago still needs a look, and is unchecked too: health counts it in both.
    assert_eq!(stands(Some(&at(2, "changed since recorded: the place last changed on 2026-11-02")), now), (Standing::NeedsALook, false));
    assert_eq!(stands(Some(&at(UNCHECKED_DAYS + 1, "missing: AWS has no secret by that id")), now), (Standing::NeedsALook, true));
    assert_eq!(stands(None, now), (Standing::Unchecked, true));
}

#[tokio::test]
async fn a_check_of_a_link_with_copies_counts_once_toward_the_agents_hour() {
    let s = store();
    let id = linked_with(&s, &[(COPIES, "aws-secret: prod/invoice-sync-replica")]);
    let cli = Cli::with(&[("prod/invoice-sync", Ok(UNCHANGED)), ("prod/invoice-sync-replica", Ok(UNCHANGED))]);
    for _ in 0..source_link::AGENT_CHECKS_PER_HOUR {
        let plan = plan_for_agent(&s, &id, "builder", Some(REASON)).unwrap();
        assert_eq!(source_link::check(&s, &cli, &plan, Via::Host).await.outcome, PRESENT);
    }
    assert!(plan_for_agent(&s, &id, "builder", Some(REASON)).unwrap_err().contains("is the limit"));
    assert_eq!((rows(&s, CHECKED).len(), rows(&s, CHECKED_COPY).len()), (6, 6));
}

#[test]
fn health_counts_links_by_their_checks_and_verify_finds_nothing_wrong_with_one() {
    let s = store();
    let id = linked_with(&s, &[]);
    create(&s, NewEntry { label: "Deploy token".into(), ..entry(&[], Some(VALUE)) }).unwrap();
    let today = chrono::NaiveDate::from_ymd_opt(2026, 11, 15).unwrap();
    let health = inspect::health(&s, Some("builder"), &seal::key(false), "test", today).unwrap();
    assert_eq!((health.entries, health.linked, health.linked_changed_or_missing, health.linked_unchecked_for_30_days), (2, 1, 0, 1));
    let verified = inspect::verify(&s, Some(&id), Some("builder"), &seal::key(false), today).unwrap();
    assert!(verified.ok, "{:?}", verified.problems);
}

#[test]
fn an_agents_change_to_what_a_link_says_is_the_persons_to_allow() {
    let s = store();
    let id = linked_with(&s, &[(READERS, "invoice-sync (lambda): at run time")]);
    let asked = UpdateRequest { id: id.clone(), label: None, fields: fields(&[(READERS, READERS_TEXT), (ROTATED, "2026-11-02")]), tags: None, reason: Some(REASON.into()) };
    let change = inspect::link_change(&s, &asked, "builder").unwrap().expect("a change to the readers is asked about");
    assert!(change.only && change.from == change.to);
    assert_eq!(change.about_from, "Read by: invoice-sync (lambda): at run time.");
    // One thing a line: the card shows the date on a line of its own.
    assert_eq!(change.about_to, "Read by: invoice-sync (lambda): at run time; release (workflow): on a tag.\nLast changed: 2026-11-02.");
    assert!(inspect::update_details(&s, &asked, "builder").unwrap_err().contains("the person is asked before it changes"));
    let updated = inspect::update_details_allowed(&s, &asked, "builder", &change).unwrap();
    assert_eq!(updated.changed, ["field readers", "field rotated"]);
    // A change to its notes alone asks nobody.
    let notes = UpdateRequest { fields: fields(&[("notes", "billing asks for it monthly")]), ..asked };
    assert_eq!(inspect::link_change(&s, &notes, "builder"), Ok(None));
}

/// The pane finds a place's check by the line as the entry writes it, so a line with no space after its kind is
/// answered as written.
#[test]
fn a_copy_is_known_by_its_line_as_the_entry_writes_it() {
    let s = store();
    let id = linked_with(&s, &[(COPIES, "aws-secret:prod/invoice-sync-replica#api.token")]);
    let known = copy_checks(&s, &s.vault_get(&id).unwrap().unwrap());
    assert_eq!(known.iter().map(|c| (c.place.as_str(), c.cannot.as_str())).collect::<Vec<_>>(), [("aws-secret:prod/invoice-sync-replica#api.token", "")]);
}

const CI_SECRET: &str = "repos/example-org/example-app/actions/secrets/EXAMPLE_TOKEN";
const CI_COLLECTION: &str = "repos/example-org/example-app/actions/secrets?per_page=1";
/// What the collection answers: an entry of a secret nobody asked about. Nothing of it may be kept.
const CI_NEIGHBOUR: &str = r#"{"total_count": 2, "secrets": [{"name": "NEIGHBOUR_SENTINEL_2291", "updated_at": "2026-02-02T00:00:00Z"}]}"#;
const CI_ENTRY: &str = r#"{"name": "EXAMPLE_TOKEN", "created_at": "2026-01-05T10:00:00Z", "updated_at": "2026-10-01T09:00:00Z", "made_up_extra": "ci-sentinel-4417"}"#;
const CI_ENTRY_LATER: &str = r#"{"name": "EXAMPLE_TOKEN", "updated_at": "2026-11-02T08:00:00Z"}"#;

/// A link to a repository's CI secret.
fn ci_place() -> Vec<(&'static str, &'static str)> {
    vec![(LINK, ONLY), (KIND, source_link::GITHUB_SECRET), (LOCATION, "example-org/example-app"), (KEY, "EXAMPLE_TOKEN")]
}

#[tokio::test]
async fn a_ci_secrets_check_says_whether_it_is_there_and_when_it_last_changed() {
    let s = store();
    let id = create(&s, NewEntry { label: "Store upload key".into(), ..entry(&[ci_place(), vec![(ROTATED, "2026-10-01")]].concat(), None) }).unwrap();
    let plan = plan_for_person(&s, &id).unwrap();
    let row = || s.vault_get(&id).unwrap().unwrap();
    for (github, outcome, why, last_changed, stands) in [
        (vec![(CI_SECRET, 200, CI_ENTRY)], PRESENT, "", Some("2026-10-01"), Standing::Checked),
        (vec![(CI_SECRET, 200, CI_ENTRY_LATER)], CHANGED, "the place last changed on 2026-11-02, after the 2026-10-01 recorded", Some("2026-11-02"), Standing::NeedsALook),
        (vec![(CI_COLLECTION, 200, CI_NEIGHBOUR)], MISSING, "GitHub has no secret by that name there", None, Standing::NeedsALook),
        (vec![], COULD_NOT_READ, "the secrets there can't be read with this computer's GitHub login", None, Standing::Checked),
    ] {
        let cli = Cli::with(&[]).github(&github);
        let checked = source_link::check(&s, &cli, &plan, Via::Host).await;
        assert_eq!((checked.outcome, checked.why.as_str(), checked.last_changed.as_deref()), (outcome, why, last_changed));
        // A CI secret has no rotation setting, and the AWS CLI is never run for one.
        assert_eq!((checked.rotation, checked.source.as_str(), cli.seen().len()), (None, "github-secret:example-org/example-app#EXAMPLE_TOKEN", 0));
        assert_eq!(standing(&s, &row()), Some((stands, false)), "{outcome}");
    }
    // Of GitHub's answer only found or not and the day are kept: nothing else of it is in the history or the answer,
    // and nothing at all of the collection's answer, which names another secret. A check is one line toward the
    // hour, however many times it asked.
    let gone = Cli::with(&[]).github(&[(CI_COLLECTION, 200, CI_NEIGHBOUR)]);
    let answer = serde_json::to_string(&source_link::check(&s, &gone, &plan, Via::Host).await).unwrap();
    assert!(answer.contains("missing") && !answer.contains("NEIGHBOUR") && !answer.contains("2026-02-02"), "{answer}");
    assert_eq!(gone.gh.asked(), [CI_SECRET, CI_COLLECTION]);
    assert_eq!(rows(&s, CHECKED).len(), 5);
    let kept = format!("{:?}", s.vault_activity_all().unwrap());
    assert!(!kept.contains("ci-sentinel") && !kept.contains("2026-01-05") && !kept.contains("NEIGHBOUR") && !kept.contains("2026-02-02"), "{kept}");

    // An environment's secret is read from the environment's own listing.
    let staged = create(&s, NewEntry { label: "Release key".into(), ..entry(&[(LINK, ONLY), (KIND, source_link::GITHUB_SECRET), (LOCATION, "example-org/example-app@production"), (KEY, "EXAMPLE_TOKEN")], None) }).unwrap();
    let endpoint = "repos/example-org/example-app/environments/production/secrets/EXAMPLE_TOKEN";
    let cli = Cli::with(&[]).github(&[(endpoint, 200, CI_ENTRY_LATER)]);
    let checked = source_link::check(&s, &cli, &plan_for_person(&s, &staged).unwrap(), Via::Host).await;
    assert_eq!((checked.outcome, checked.last_changed.as_deref(), cli.gh.asked()), (PRESENT, Some("2026-11-02"), vec![endpoint.to_string()]));
}

#[tokio::test]
async fn an_entry_that_holds_a_value_and_names_a_ci_secret_is_described_not_compared() {
    let s = store();
    let fields = [(KIND, source_link::GITHUB_SECRET), (LOCATION, "example-org/example-app"), (KEY, "EXAMPLE_TOKEN"), (COPIES, "github-secret: example-org/example-app@production#EXAMPLE_TOKEN")];
    let id = create(&s, entry(&fields, Some(VALUE))).unwrap();
    let plan = plan_for_agent(&s, &id, "builder", Some(REASON)).unwrap();
    assert!(plan.describes() && !plan.only);
    assert_eq!(plan.github_places().len(), 2);
    let cli = Cli::with(&[]).github(&[(CI_SECRET, 200, CI_ENTRY), ("repos/example-org/example-app/environments/production/secrets?per_page=1", 200, CI_NEIGHBOUR)]);
    let checked = source_link::check(&s, &cli, &plan, Via::Host).await;
    // GitHub gives no value back, so nothing is compared: the check says the secret is there and when it changed.
    assert_eq!((checked.outcome, checked.last_changed.as_deref()), (PRESENT, Some("2026-10-01")));
    assert_eq!((checked.copies[0].outcome, checked.copies[0].why.as_str()), (MISSING, "GitHub has no secret by that name there"));
    let said = serde_json::to_string(&checked).unwrap() + &format!("{:?}", s.vault_activity_all().unwrap());
    assert!(!said.contains(VALUE));

    // An entry whose place is in AWS is compared with its value, and its other places are not read then.
    let in_aws = create(&s, NewEntry { label: "Deploy token".into(), ..entry(&[(KIND, AWS_SECRET), (LOCATION, "prod/deploy"), (COPIES, "github-secret: example-org/example-app#EXAMPLE_TOKEN")], Some(VALUE)) }).unwrap();
    let plan = plan_for_person(&s, &in_aws).unwrap();
    assert!(!plan.describes() && plan.github_places().is_empty());
}

#[tokio::test]
async fn a_check_that_may_not_start_a_ci_secrets_read_answers_with_the_stored_result() {
    let s = store();
    let id = create(&s, NewEntry { label: "Store upload key".into(), ..entry(&ci_place(), None) }).unwrap();
    let held = |plan: CheckPlan| CheckPlan { no_github: Some("Made-up reason, and what to do.".into()), ..plan };
    let cli = Cli::with(&[]).github(&[(CI_SECRET, 200, CI_ENTRY)]);

    // Never checked: nothing is run, nothing is written, and the answer says so.
    let first = source_link::check(&s, &cli, &held(plan_for_agent(&s, &id, "boxed", Some(REASON)).unwrap()), Via::Host).await;
    assert_eq!((first.outcome, first.why.as_str(), first.stored.clone()), (NOT_STARTED, "Made-up reason, and what to do. It has not been checked yet.", None));
    assert!(cli.gh.asked().is_empty() && rows(&s, CHECKED).is_empty());

    // After the person's check the same answer carries what that check found and when.
    let by_person = source_link::check(&s, &cli, &plan_for_person(&s, &id).unwrap(), Via::Host).await;
    assert_eq!(by_person.outcome, PRESENT);
    let again = source_link::check(&s, &cli, &held(plan_for_agent(&s, &id, "boxed", Some(REASON)).unwrap()), Via::Host).await;
    assert_eq!((again.outcome, again.why.as_str()), (NOT_STARTED, "Made-up reason, and what to do. The newest stored result is in `stored`."));
    assert_eq!(again.stored.as_ref().map(|c| c.outcome.as_str()), Some(PRESENT));
    assert_eq!((cli.gh.asked().len(), rows(&s, CHECKED).len()), (1, 1), "the held checks ran nothing and wrote nothing");
    // A check that wrote no line does not count toward the agents' hour.
    for _ in 0..source_link::AGENT_CHECKS_PER_HOUR + 1 {
        let plan = held(plan_for_agent(&s, &id, "boxed", Some(REASON)).unwrap());
        assert_eq!(source_link::check(&s, &cli, &plan, Via::Host).await.outcome, NOT_STARTED);
    }

    // A CI secret among the other places of an AWS link: the AWS place is described, and the copy is held.
    let mixed = linked_with(&s, &[(COPIES, "github-secret: example-org/example-app#EXAMPLE_TOKEN")]);
    let cli = Cli::with(&[("prod/invoice-sync", Ok(UNCHANGED))]).github(&[(CI_SECRET, 200, CI_ENTRY)]);
    let checked = source_link::check(&s, &cli, &held(plan_for_agent(&s, &mixed, "boxed", Some(REASON)).unwrap()), Via::Host).await;
    assert_eq!((checked.outcome, checked.copies[0].outcome, checked.copies[0].stored.clone()), (PRESENT, NOT_STARTED, None));
    assert!(cli.gh.asked().is_empty());
    assert!(rows(&s, CHECKED_COPY).iter().all(|r| r.entry_id != mixed));
}

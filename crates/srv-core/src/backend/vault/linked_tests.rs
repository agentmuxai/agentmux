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
use crate::backend::vault::inspect::{self, UpdateRequest};
use crate::backend::vault::seal;
use crate::backend::vault::source_link::{plan_for_agent, plan_for_person, AgentLink, CHANGED, CHECKED, COULD_NOT_READ, KEY, KIND, LOCATION, MISSING, PRESENT, PROFILE, REGION};
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

/// A stand-in AWS CLI: each secret it knows answers with its description, and any other with what AWS said.
struct Cli {
    known: Vec<(&'static str, Result<&'static str, &'static str>)>,
    /// The operation and the secret of every run.
    seen: Mutex<Vec<(String, String)>>,
}

impl Cli {
    fn with(known: &[(&'static str, Result<&'static str, &'static str>)]) -> Self {
        Self { known: known.to_vec(), seen: Mutex::new(Vec::new()) }
    }

    fn seen(&self) -> Vec<(String, String)> {
        self.seen.lock().unwrap().clone()
    }
}

impl Runner for Cli {
    fn which(&self, _program: &str) -> Option<PathBuf> {
        Some(PathBuf::from("/bin/aws"))
    }

    async fn run(&self, spec: RunSpec) -> Result<RunOutput, RunError> {
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
    let cli = Cli::with(&[("prod/invoice-sync", Ok(UNCHANGED)), ("prod/invoice-sync-replica", Ok(CHANGED_LATER))]);
    let checked = source_link::check(&s, &cli, &plan, Via::Host).await;
    assert_eq!((checked.outcome, checked.why.as_str(), checked.source.as_str()), (PRESENT, "", "aws-secret:prod/invoice-sync#api.token"));
    assert_eq!((checked.last_changed.as_deref(), checked.rotation), (Some("2026-09-30"), Some(true)));
    // Each other place is in the answer; the one in AWS was described under the entry's own region, and the CI
    // secret, which nothing here can describe, ran nothing.
    let copies: Vec<(&str, &str)> = checked.copies.iter().map(|c| (c.place.as_str(), c.outcome)).collect();
    assert_eq!(copies, [("github-secret: example-org/example-app#EXAMPLE_TOKEN", COULD_NOT_READ), ("aws-secret: prod/invoice-sync-replica#api.token", CHANGED)]);
    assert_eq!(checked.copies[1].why, "the place last changed on 2026-11-02, after the 2026-10-01 recorded");
    let described: Vec<(String, String)> = [("describe-secret", "prod/invoice-sync"), ("describe-secret", "prod/invoice-sync-replica")].iter().map(|(o, i)| (o.to_string(), i.to_string())).collect();
    assert_eq!(cli.seen(), described);

    // A line for the entry's own place and one for the copy that was asked, each naming its place.
    let own = rows(&s, CHECKED);
    assert_eq!(own.iter().map(|r| (r.target.as_str(), r.outcome.as_str(), r.actor.as_str(), r.reason.as_str())).collect::<Vec<_>>(), [("aws-secret:prod/invoice-sync#api.token", PRESENT, "builder", REASON)]);
    let other = rows(&s, CHECKED_COPY);
    assert_eq!(other.iter().map(|r| r.target.as_str()).collect::<Vec<_>>(), ["aws-secret:prod/invoice-sync-replica#api.token"]);
    assert!(other[0].outcome.starts_with("changed since recorded: the place last changed on 2026-11-02"));
    // The newest check is the entry's own place's, whatever its copies' lines say.
    let row = s.vault_get(&id).unwrap().unwrap();
    assert_eq!(source_link::last_check(&s, &row, &Link::of_row(&row).unwrap()).map(|c| c.outcome), Some(PRESENT.to_string()));
    assert_eq!(standing(&s, &row), Some((Standing::Checked, false)));
    // What the pane shows of each other place: the newest check of the one in AWS, and of the CI secret that no
    // check can describe it, which until now was in the asking agent's answer only.
    let known = copy_checks(&s, &row);
    let said: Vec<(&str, &str, bool)> = known.iter().map(|c| (c.place.as_str(), c.cannot.as_str(), c.checked.is_some())).collect();
    assert_eq!(said, [("github-secret: example-org/example-app#EXAMPLE_TOKEN", NOT_FROM_HERE, false), ("aws-secret: prod/invoice-sync-replica#api.token", "", true)]);
    assert!(known[1].checked.as_ref().unwrap().outcome.starts_with("changed since recorded: the place last changed on 2026-11-02"));
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

    // A place that isn't in AWS can't be described from here: the check says so and runs nothing.
    let ci = create(&s, NewEntry { label: "Store upload key".into(), ..entry(&[(LINK, ONLY), (KIND, source_link::GITHUB_SECRET), (LOCATION, "example-org/example-app"), (KEY, "STORE_KEY"), (COPIES, "aws-secret: prod/store-key")], None) }).unwrap();
    let cli = Cli::with(&[]);
    let unread = source_link::check(&s, &cli, &plan_for_person(&s, &ci).unwrap(), Via::Host).await;
    assert_eq!((unread.outcome, unread.why.as_str(), cli.seen().len()), (COULD_NOT_READ, NOT_FROM_HERE, 0));
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

// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Expiry dates and their reminders
//! (docs/specs/SPEC_VAULT_GENERATE_CAPTURE_AND_BROWSER_SIGNINS_2026_10_28.md §15.2).
//!
//! An entry's `expires` field is a plain date (`YYYY-MM-DD`): no secret, so
//! srv reads it on a schedule without opening anything. At 14 days, 3 days and
//! on the day the person is notified, once each: the reminder is a line in the
//! Vault's activity log, which is what says it was sent, across restarts. The
//! first reminder for a date also files a task to rotate the value, for the
//! agent that made it when an agent did.
//!
//! An expired entry stays usable, so a reminder never breaks a running job;
//! each use of one is recorded with "(expired)".

use std::sync::Arc;
use std::time::Duration;

use agentmux_common::time::now_ms;
use chrono::NaiveDate;

use super::{kinds, record, PERSON};
use crate::backend::mps::Broker;
use crate::backend::storage::store::Store;
use crate::backend::storage::vault::VaultRow;
use crate::backend::storage::work_board::{requested, NewCard, WorkEvent};
use crate::backend::storage::work_queue::{work_state, WorkItem};

/// The field that holds an entry's expiry date.
pub const FIELD: &str = "expires";
/// Who the activity log and the task name for the reminders.
pub const ACTOR: &str = "Vault";
pub const REMINDED: &str = "reminded";
/// The rotation task's `kind`.
pub const TASK_KIND: &str = "vault-rotation";
/// In a reminder's activity line once its rotation task is filed.
const TASK_MARK: &str = "; rotation task #";
/// How often srv looks, besides at start.
const EVERY: Duration = Duration::from_secs(6 * 60 * 60);
/// The actions that make or replace an entry's value: the agent behind the
/// latest one is asked to rotate it.
const MADE: &[&str] = &["generated", "captured", "rotated", "replaced", "created"];

/// A date as the Vault keeps it, or why it isn't one.
pub fn parse(text: &str) -> Result<NaiveDate, String> {
    let text = text.trim();
    if text.len() != 10 {
        return Err(format!("expires is a date as YYYY-MM-DD, not {:?}", clip(text)));
    }
    NaiveDate::parse_from_str(text, "%Y-%m-%d").map_err(|_| format!("expires is a date as YYYY-MM-DD, not {:?}", clip(text)))
}

/// A date another manager or an older backup wrote: `YYYY-MM-DD` as it is, and a
/// month (`06/2028`, `6/2028`, `2028-06`) as its last day. None for anything else.
pub fn normalize(text: &str) -> Option<String> {
    let text = text.trim();
    if let Ok(d) = parse(text) {
        return Some(d.to_string());
    }
    let (y, m) = match text.split_once('/') {
        Some((m, y)) => (y.trim(), m.trim()),
        None => text.split_once('-')?,
    };
    let (y, m): (i32, u32) = (y.parse().ok().filter(|_| y.len() == 4)?, m.parse().ok()?);
    let first_of_next = if m == 12 { NaiveDate::from_ymd_opt(y + 1, 1, 1) } else { NaiveDate::from_ymd_opt(y, m + 1, 1) };
    first_of_next.filter(|_| (1..=12).contains(&m)).and_then(|d| d.pred_opt()).map(|d| d.to_string())
}

/// An imported entry's `expires`, made a date the Vault keeps: normalized when it
/// can be, else moved into the entry's notes, so no entry is refused for it and
/// no text is lost.
pub fn settle_imported(kind: &kinds::Kind, fields: &mut std::collections::BTreeMap<String, String>) {
    let Some(text) = fields.remove(FIELD) else { return };
    match normalize(&text) {
        Some(date) => {
            fields.insert(FIELD.to_string(), date);
        }
        None if kind.fields.contains(&"notes") => {
            let line = format!("Expires: {text}");
            let notes = fields.remove("notes").filter(|n| !n.is_empty()).map(|n| format!("{n}\n{line}")).unwrap_or(line);
            fields.insert("notes".to_string(), notes);
        }
        None => {}
    }
}

fn clip(text: &str) -> String {
    text.chars().take(40).collect()
}

pub fn today() -> NaiveDate {
    chrono::Local::now().date_naive()
}

/// An entry's expiry date, if its kind has one and it holds a date.
pub fn of(row: &VaultRow) -> Option<NaiveDate> {
    if !kinds::kind(&row.kind).is_some_and(|k| k.fields.contains(&FIELD)) {
        return None;
    }
    let fields: std::collections::BTreeMap<String, String> = serde_json::from_str(&row.fields).ok()?;
    parse(fields.get(FIELD)?).ok()
}

/// Whether an entry is past its date: usable still, but its uses say so.
pub fn expired(row: &VaultRow, today: NaiveDate) -> bool {
    of(row).is_some_and(|d| d < today)
}

/// The three reminders.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    TwoWeeks,
    ThreeDays,
    OnTheDay,
}

impl Stage {
    /// The reminder due `days_left` before the date, if any: the nearest one, so
    /// an entry saved 2 days before its date gets the 3-day reminder, not three.
    pub fn due(days_left: i64) -> Option<Stage> {
        match days_left {
            d if d <= 0 => Some(Stage::OnTheDay),
            1..=3 => Some(Stage::ThreeDays),
            4..=14 => Some(Stage::TwoWeeks),
            _ => None,
        }
    }

    fn words(self) -> &'static str {
        match self {
            Stage::TwoWeeks => "14 days before",
            Stage::ThreeDays => "3 days before",
            Stage::OnTheDay => "on the day",
        }
    }
}

/// The activity line's target that says this reminder for this date was sent.
/// The date is in it, so a new date starts its reminders again.
pub fn reminder_key(date: NaiveDate, stage: Stage) -> String {
    format!("expires {date}, {}", stage.words())
}

/// What the person reads.
pub fn notice(date: NaiveDate, days_left: i64) -> String {
    match days_left {
        d if d < 0 => format!("Expired on {date}. It still works where it is used, until it's replaced."),
        0 => format!("Expires today, {date}."),
        1 => format!("Expires tomorrow, {date}."),
        d => format!("Expires in {d} days, on {date}."),
    }
}

/// A reminder to send.
#[derive(Debug, Clone, PartialEq)]
pub struct Due {
    pub id: String,
    pub label: String,
    pub date: NaiveDate,
    pub days_left: i64,
    pub stage: Stage,
    /// No rotation task was filed for this date yet: this reminder files it, so
    /// one that couldn't be filed is tried again at the next reminder.
    pub first: bool,
    /// The agent that made or last replaced the value, if one did.
    pub agent: Option<String>,
}

/// The reminders due today that haven't been sent.
pub fn due(store: &Store, today: NaiveDate) -> Result<Vec<Due>, String> {
    let mut out = Vec::new();
    for row in store.vault_list().map_err(|e| e.to_string())? {
        let Some(date) = of(&row) else { continue };
        let days_left = (date - today).num_days();
        let Some(stage) = Stage::due(days_left) else { continue };
        // By action, so a key's many uses never push these lines out. Three
        // reminders a date: an entry's whole history of them is short.
        let reminded = store.vault_activity_for_entry(&row.id, &[REMINDED], row.created_at, 10_000).map_err(|e| e.to_string())?;
        let sent_for_date = format!("expires {date}, ");
        let sent: Vec<&str> = reminded.iter().filter(|l| l.target.starts_with(&sent_for_date)).map(|l| l.target.as_str()).collect();
        if sent.iter().any(|t| t.starts_with(&reminder_key(date, stage))) {
            continue;
        }
        // The latest line that made the value.
        let made = store.vault_activity_for_entry(&row.id, MADE, row.created_at, 1).map_err(|e| e.to_string())?;
        let agent = made.into_iter().next().map(|l| l.actor).filter(|a| a != PERSON && !a.is_empty());
        out.push(Due { id: row.id, label: row.label, date, days_left, stage, first: !sent.iter().any(|t| t.contains(TASK_MARK)), agent });
    }
    Ok(out)
}

pub fn task_title(d: &Due) -> String {
    format!("Rotate {}: it expires on {}", d.label, d.date)
}

pub fn task_body(d: &Due) -> String {
    format!(
        "The Vault entry `{}` ({}) expires on {}.\n\n\
         Make a new value where it is issued, put it where the old one is used, and save it in the entry, \
         without seeing it: `Vault` `generate` makes one the Vault keeps, and `Terminal` `run` with `secrets`, \
         `stdin_secret` or `secret_files` hands it to a command. Then set the entry's new expiry date.\n\n\
         The old value keeps working until it expires, so nothing breaks before then.",
        d.id, d.label, d.date
    )
}

/// Files the rotation task, as `kanban.act` `create` does; returns its number.
pub fn file_task(work: &Store, d: &Due, now: i64) -> Result<i64, String> {
    let item = WorkItem {
        id: uuid::Uuid::new_v4().to_string(),
        title: task_title(d),
        payload: task_body(d),
        kind: TASK_KIND.to_string(),
        target_agent: d.agent.clone().unwrap_or_default(),
        target_group: String::new(),
        priority: 0,
        state: String::new(),
        claimed_by: String::new(),
        target_agent_uid: String::new(),
        claimed_by_uid: String::new(),
        claim_expires: None,
        attempts: 0,
        max_attempts: 3,
        created_by: ACTOR.to_string(),
        created_by_uid: String::new(),
        created_at: now,
        updated_at: now,
        not_before: None,
        result: String::new(),
    };
    work.work_queue_enqueue(&item).map_err(|e| e.to_string())?;
    let card = NewCard {
        review_required: false,
        requested_by: ACTOR.to_string(),
        requested_by_kind: requested::OTHER.to_string(),
        filed_by: ACTOR.to_string(),
        ..Default::default()
    };
    let num = work.work_card_create(&item.id, &card, now).map_err(|e| e.to_string())?;
    let event = WorkEvent {
        work_id: item.id.clone(),
        at: now,
        actor: ACTOR.to_string(),
        actor_kind: "system".to_string(),
        kind: "created".to_string(),
        to_state: work_state::OPEN.to_string(),
        ..Default::default()
    };
    work.work_event_add(&event).map_err(|e| e.to_string())?;
    Ok(num)
}

/// What one look sent: the notifications to show, and whether a task was filed.
pub struct Sent {
    pub notices: Vec<(Due, String)>,
    pub filed: bool,
}

/// Records each reminder due and files the rotation tasks. The notifications
/// are the caller's to show: the record is written first, so a crash between
/// the two never repeats a reminder.
pub fn send(store: &Store, work: Option<&Store>, today: NaiveDate) -> Result<Sent, String> {
    let mut sent = Sent { notices: Vec::new(), filed: false };
    for d in due(store, today)? {
        let mut target = reminder_key(d.date, d.stage);
        if d.first {
            if let Some(work) = work {
                match file_task(work, &d, now_ms()) {
                    Ok(num) => {
                        sent.filed = true;
                        let whom = d.agent.as_deref().map(|a| format!(" for {a}")).unwrap_or_default();
                        target.push_str(&format!("{TASK_MARK}{num}{whom}"));
                    }
                    Err(e) => tracing::warn!(entry = %d.id, error = %e, "vault: couldn't file a rotation task"),
                }
            }
        }
        record(store, &d.id, &d.label, ACTOR, REMINDED, &target, "done");
        let text = notice(d.date, d.days_left);
        sent.notices.push((d, text));
    }
    Ok(sent)
}

/// The notification's key: one per entry.
pub fn notify_key(id: &str) -> String {
    format!("vault:{id}")
}

/// Most notifications one look shows; more entries due share one.
pub const MAX_TOASTS: usize = 3;

/// One notification to show: its key, the name its title gives, and its text.
#[derive(Debug, PartialEq)]
pub struct Toast {
    pub key: String,
    pub name: String,
    pub body: String,
}

/// The notifications for one look's reminders: one each, or, past
/// `MAX_TOASTS`, one for them all, so an import never floods the screen.
pub fn toasts(notices: &[(Due, String)]) -> Vec<Toast> {
    if notices.len() <= MAX_TOASTS {
        return notices.iter().map(|(d, text)| Toast { key: notify_key(&d.id), name: d.label.clone(), body: text.clone() }).collect();
    }
    let named: Vec<&str> = notices.iter().take(MAX_TOASTS).map(|(d, _)| d.label.as_str()).collect();
    let body = format!(
        "{}, and {} more expire within 14 days, or have expired. The Vault's Expiring soon filter lists them.",
        named.join(", "),
        notices.len() - named.len()
    );
    vec![Toast { key: "vault:expiring".into(), name: format!("{} entries", notices.len()), body }]
}

/// Looks at start and every 6 hours. A reminder is recorded as sent, so a look
/// waits until it can be shown: for the notification router, which starts with
/// the first window, and past a pause or quiet hours, which would drop it.
pub async fn run(store: Arc<Store>, work: Arc<Store>, broker: Arc<Broker>) {
    loop {
        match crate::backend::notify::router::get(&broker) {
            Some(router) if !router.holds_back_attention() => {}
            _ => {
                tokio::time::sleep(Duration::from_secs(15 * 60)).await;
                continue;
            }
        }
        let (s, w) = (store.clone(), work.clone());
        match tokio::task::spawn_blocking(move || send(&s, Some(&w), today())).await {
            Ok(Ok(sent)) => {
                if sent.filed {
                    broker.publish_event::<crate::backend::mps_events::WorkQueueChanged>(&(), vec![], 0);
                }
                if !sent.notices.is_empty() {
                    // The Activity list has new lines.
                    let event = crate::backend::mps_events::VaultEvent {
                        requests: super::requests::requests().list(),
                        uses: super::uses::uses().list(),
                        changed_ms: agentmux_common::time::now_ms_u64(),
                    };
                    broker.publish_event::<crate::backend::mps_events::Vault>(&event, vec![], 0);
                }
                for (d, _) in &sent.notices {
                    tracing::info!(entry = %d.id, date = %d.date, "vault: expiry reminder");
                }
                if let Some(router) = crate::backend::notify::router::get(&broker) {
                    for t in toasts(&sent.notices) {
                        let router = router.clone();
                        let _ = tokio::task::spawn_blocking(move || router.vault_expiring(&t.key, &t.name, t.body)).await;
                    }
                }
            }
            Ok(Err(e)) => tracing::warn!(error = %e, "vault: couldn't check expiry dates"),
            Err(e) => tracing::warn!(error = %e, "vault: the expiry check stopped"),
        }
        tokio::time::sleep(EVERY).await;
    }
}

#[cfg(test)]
#[path = "expiry_tests.rs"]
mod tests;

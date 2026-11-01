// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The person's approval of a committing click (browser spec §5.4): a click that
//! submits a form, or a button named like Send, Pay or Delete, waits for them in
//! its pane's banner. `act` clicks and `Browser` `capture` of a download use it.

use axum::http::StatusCode;
use serde_json::json;

use super::browser_actions::How;
use super::ui_handlers::banner_for;
use super::AppState;

/// Why a committing click wasn't approved: the banner couldn't be raised, or the
/// person answered (how, when the Log says so) no, too late, or took the pane over.
pub(crate) enum ClickRefused {
    NotAsked(StatusCode, String),
    Asked(Option<How>, StatusCode, String),
}

/// Asks the person to approve the committing click `ask` describes, in its pane's
/// banner (spec §5.4). `Ok` holds the pane locked until it is dropped, so the form
/// can't change between what they approved and what is sent; make the approved
/// click before dropping it. The banner shows the summary, not the fingerprint of
/// everything the click sends (hidden controls included): that stays in `ask`, for
/// the host's check before the approved click.
pub(crate) async fn approve_click(
    state: &AppState,
    block_id: &str,
    agent: &str,
    ask: &serde_json::Value,
) -> Result<(super::browser_attention::Waiting, std::time::Instant), ClickRefused> {
    use super::browser_attention::Answer;
    let (banner_at, in_window) = banner_for(block_id);
    let mut banner = ask.clone();
    if let Some(b) = banner.as_object_mut() {
        b.remove("fingerprint");
        b.insert("window".to_string(), json!(in_window));
    }
    let (owner_before, asked_at) = (super::browser_owner::owner_of(&banner_at), std::time::Instant::now());
    let (answer, waiting) = super::browser_attention::ask(
        state,
        &banner_at,
        agent,
        super::browser_attention::Kind::Approval,
        banner,
        std::time::Duration::from_secs(10 * 60),
    )
    .await
    .map_err(|e| ClickRefused::NotAsked(e.status(), e.to_string()))?;
    match answer {
        // The user may have taken the pane over while the banner was up:
        // their Take over wins over an Approve that comes after it.
        Answer::Yes | Answer::YesAlways if super::browser_owner::owner_of(&banner_at) != owner_before => Err(ClickRefused::Asked(
            None,
            StatusCode::FORBIDDEN,
            "the user took over this pane, so the approved click wasn't made".to_string(),
        )),
        Answer::Yes | Answer::YesAlways => {
            super::browser_dialog::note_approved_click(block_id, ask);
            Ok((waiting, asked_at))
        }
        Answer::Cancelled => Err(ClickRefused::Asked(
            Some(How::Declined),
            StatusCode::FORBIDDEN,
            "the user didn't approve this action: don't retry it; ask them what to do".to_string(),
        )),
        Answer::TimedOut => Err(ClickRefused::Asked(
            Some(How::TimedOut),
            StatusCode::REQUEST_TIMEOUT,
            "the user didn't answer the approval within 10 minutes; ask them in chat before trying again".to_string(),
        )),
    }
}

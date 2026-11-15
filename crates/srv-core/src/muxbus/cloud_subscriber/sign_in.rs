// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What the subscriber's loop asks of the stored sign-in: whether it can be used
//! or refreshed, whether a refresh came back for the same account, and a wait for
//! a new sign-in.

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc;

use super::supervisor::beat;
use super::CtrlMsg;
use crate::backend::storage::store::Store;

/// [`super::load_valid_token`] for the subscriber's own loop, which stamps its progress once the
/// call returns. Only code that runs in the loop may stamp: the helper is shared with callers
/// elsewhere, and a stamp from them would hide a stalled loop.
pub(super) async fn token_for_loop(mstore: &Arc<Store>, scheduler: &crate::broker::RefreshScheduler) -> Option<String> {
    let token = super::load_valid_token(mstore, scheduler).await;
    beat();
    token
}

/// A loaded sign-in that can be used now or refreshed: a valid access token,
/// or a refresh token (all a torn sign-in keeps, see `storage::muxbus`).
pub(super) fn usable_or_refreshable(c: &crate::muxbus::MuxBusCredentials) -> bool {
    c.is_valid() || !c.refresh_token.is_empty()
}

/// Whether a refresh came back for the account the saved sign-in names. A
/// torn set could pair a refresh token with another account's row; a row
/// with no account recorded (an older sign-in) can't be checked.
pub(super) fn refreshed_same_account(current: &crate::muxbus::MuxBusCredentials, refreshed: &crate::muxbus::MuxBusCredentials) -> bool {
    if current.user_sub.is_empty() {
        return true;
    }
    let sub = crate::muxbus::pkce::token_sub(&refreshed.id_token);
    let sub = if sub.is_empty() { crate::muxbus::pkce::token_sub(&refreshed.access_token) } else { sub };
    sub == current.user_sub
}

/// Park until a sign-in (`ReloadToken`) or `recheck` elapses, whichever comes
/// first. AddAgent/RemoveAgent drain without waking it: they already updated
/// the `agents` set the next connection subscribes from.
pub(super) async fn wait_for_sign_in(ctrl_rx: &mut mpsc::UnboundedReceiver<CtrlMsg>, recheck: Duration) {
    beat();
    let sleep = tokio::time::sleep(recheck);
    tokio::pin!(sleep);
    loop {
        tokio::select! {
            _ = &mut sleep => return,
            msg = ctrl_rx.recv() => match msg {
                Some(CtrlMsg::ReloadToken) => return,
                Some(CtrlMsg::AddAgent(_) | CtrlMsg::RemoveAgent(_)) => {}
                // Closed: nothing can wake us early; wait out the recheck.
                None => {
                    (&mut sleep).await;
                    return;
                }
            }
        }
    }
}

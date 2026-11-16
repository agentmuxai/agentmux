// Copyright 2025, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What a subprocess controller leaves for the one that replaces it.
//!
//! A resync that needs a different controller (a runtime-config or controller-type change) stops the old
//! one with `stop_for_replace` and builds a new one for the same block. The messages the old one had accepted
//! are not lost with it:
//!
//! - what was queued behind the turn the replace ended is run by the replacement, which starts a turn from
//!   the first of them as soon as it is registered ([`SubprocessController::run_queue_left_by_replaced`]);
//! - what a user stop held (`held_after_stop`) stays held, and is queued behind the next turn that is accepted.
//!
//! Both wait in [`LEFT_FOR_REPLACEMENT`] between the old controller's last act and the new one's construction.
//! An entry nobody takes (the block is deleted, or its replacement is a controller of another type) is dropped
//! when the block is deleted ([`forget_left_for_replacement`]) and after [`LEFT_FOR_REPLACEMENT_TTL`].

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::{SubprocessController, SubprocessSpawnConfig};
use crate::backend::runtime_adapter::RuntimeAdapter;

/// How long an entry waits for its replacement. A resync builds that within the same call, so this only
/// bounds an entry whose replacement never comes.
const LEFT_FOR_REPLACEMENT_TTL: Duration = Duration::from_secs(300);

/// How long the replacement waits for the replaced controller's killed turn to be gone before it starts its own.
const OLD_TURN_EXIT_WAIT: Duration = Duration::from_secs(10);

/// The container a container turn ran in and the command it ran: a queued message's own CLI arguments were taken
/// into that command when it was queued, so the queue is run with it again, not through the adapter.
#[derive(Clone)]
pub(super) struct ContainerContext {
    cm: crate::backend::container::ContainerManager,
    container_name: String,
    base_cmd: Vec<String>,
}

/// What one replaced controller leaves for the next one of its block.
pub(super) struct LeftForReplacement {
    /// Held by a user stop; stay held.
    pub(super) held: VecDeque<SubprocessSpawnConfig>,
    /// Queued behind the turn the replace ended; the replacement runs them.
    pub(super) queued: VecDeque<SubprocessSpawnConfig>,
    /// The session the old controller was on: a queued message continues it.
    pub(super) session_id: Option<String>,
    /// How the last message was launched (host process or container exec), to launch the queued ones the same way.
    pub(super) adapter: Option<Arc<dyn RuntimeAdapter>>,
    /// The old controller's run lock: set while its killed turn is still winding down.
    pub(super) old_run_lock: Option<Arc<AtomicBool>>,
    /// For a container agent: how its queued messages are launched.
    pub(super) container: Option<ContainerContext>,
    left_at: Instant,
}

static LEFT_FOR_REPLACEMENT: Mutex<Option<HashMap<String, LeftForReplacement>>> = Mutex::new(None);

fn drop_expired(map: &mut HashMap<String, LeftForReplacement>) {
    map.retain(|_, left| left.left_at.elapsed() < LEFT_FOR_REPLACEMENT_TTL);
}

/// The entry [`SubprocessController::hand_over_held_to_replacement`] left for `block_id`'s next controller.
pub(super) fn take_left_for_replacement(block_id: &str) -> Option<LeftForReplacement> {
    let mut guard = LEFT_FOR_REPLACEMENT.lock().unwrap();
    let map = guard.as_mut()?;
    drop_expired(map);
    map.remove(block_id)
}

/// Drop what a replaced controller left for `block_id`: the block is gone, so nothing will take it.
pub fn forget_left_for_replacement(block_id: &str) {
    if let Some(map) = LEFT_FOR_REPLACEMENT.lock().unwrap().as_mut() {
        map.remove(block_id);
    }
}

#[cfg(test)]
pub(crate) fn left_for_replacement_count(block_id: &str) -> usize {
    LEFT_FOR_REPLACEMENT.lock().unwrap().as_ref().map_or(0, |m| usize::from(m.contains_key(block_id)))
}

#[cfg(test)]
pub(crate) fn age_left_for_replacement(block_id: &str, by: Duration) {
    if let Some(left) = LEFT_FOR_REPLACEMENT.lock().unwrap().as_mut().and_then(|m| m.get_mut(block_id)) {
        left.left_at -= by;
    }
}

impl SubprocessController {
    /// How the next queued message is launched: remembered from the last message that was accepted.
    pub fn remember_adapter(&self, adapter: Arc<dyn RuntimeAdapter>) {
        *self.adapter.lock().unwrap() = Some(adapter);
    }

    /// A container turn is starting or queuing: remember what it runs in and with.
    pub(super) fn remember_container_context(&self, cm: &crate::backend::container::ContainerManager, container_name: &str, base_cmd: &[String]) {
        *self.container_context.lock().unwrap() =
            Some(ContainerContext { cm: cm.clone(), container_name: container_name.to_string(), base_cmd: base_cmd.to_vec() });
    }

    /// `stop_for_replace`: end the turn, but keep what was queued behind it for the replacement. Unlike `stop`
    /// this is not a user's stop, so nothing is held: the messages were accepted and are to run.
    pub(super) fn stop_keeping_queue_for_replacement(&self, new_status: &str) -> Result<(), String> {
        {
            let mut inner = self.inner.lock().unwrap();
            let queued = std::mem::take(&mut inner.pending_messages);
            if !queued.is_empty() {
                tracing::info!(
                    block_id = %self.block_id,
                    queued = queued.len(),
                    "controller replaced: messages queued behind its turn go to the new one"
                );
            }
            inner.queued_for_replacement.extend(queued);
        }
        self.stop_subprocess(true)?;
        let mut inner = self.inner.lock().unwrap();
        if inner.proc_status != new_status {
            Self::set_status(&mut inner, new_status);
        }
        Ok(())
    }

    /// This controller is being replaced: what it holds goes to the controller that takes its place.
    pub(crate) fn hand_over_held_to_replacement(&self) {
        let (held, queued, session_id) = {
            let mut inner = self.inner.lock().unwrap();
            let mut queued = std::mem::take(&mut inner.queued_for_replacement);
            queued.extend(std::mem::take(&mut inner.pending_messages));
            (std::mem::take(&mut inner.held_after_stop), queued, inner.session_id.clone())
        };
        if held.is_empty() && queued.is_empty() {
            return;
        }
        tracing::info!(
            block_id = %self.block_id,
            held = held.len(),
            queued = queued.len(),
            "controller replaced: its messages go to the new one"
        );
        let adapter = self.adapter.lock().unwrap().clone();
        let mut guard = LEFT_FOR_REPLACEMENT.lock().unwrap();
        let map = guard.get_or_insert_with(HashMap::new);
        drop_expired(map);
        let left = map.entry(self.block_id.clone()).or_insert_with(|| LeftForReplacement {
            held: VecDeque::new(),
            queued: VecDeque::new(),
            session_id: None,
            adapter: None,
            old_run_lock: None,
            container: None,
            left_at: Instant::now(),
        });
        left.held.extend(held);
        left.queued.extend(queued);
        left.session_id = session_id.or_else(|| left.session_id.take());
        left.adapter = adapter.or_else(|| left.adapter.take());
        left.old_run_lock = Some(Arc::clone(&self.run_lock));
        left.container = self.container_context.lock().unwrap().clone().or_else(|| left.container.take());
        left.left_at = Instant::now();
    }

    /// A replacement built from what the old controller left: start a turn from the first message it had queued
    /// (the rest run behind it), once the old controller's killed turn is gone. Without a way to launch
    /// (no adapter was remembered, or there is no async runtime), the messages are held instead, to run behind
    /// the next turn that is accepted: never dropped.
    pub(crate) fn run_queue_left_by_replaced(self: &Arc<Self>) {
        let adapter = self.adapter.lock().unwrap().clone();
        let old_run_lock = self.left_old_run_lock.lock().unwrap().take();
        let container = self.container_context.lock().unwrap().clone();
        let handle = tokio::runtime::Handle::try_current().ok();
        let (Some(adapter), Some(handle)) = (adapter, handle) else {
            let mut inner = self.inner.lock().unwrap();
            if !inner.pending_messages.is_empty() {
                tracing::warn!(
                    block_id = %self.block_id,
                    queued = inner.pending_messages.len(),
                    "replacement cannot launch the replaced controller's queue now: held for the next turn"
                );
                let queued = std::mem::take(&mut inner.pending_messages);
                inner.held_after_stop.extend(queued);
            }
            return;
        };
        if self.inner.lock().unwrap().pending_messages.is_empty() {
            return;
        }
        // The run lock is held while the old turn winds down, so a message that arrives meanwhile queues behind
        // the handed-over ones instead of running ahead of them. (It was just built: the lock is free.)
        let holding = self.try_lock_run();
        let ctrl = Arc::clone(self);
        handle.spawn(async move {
            if let Some(lock) = old_run_lock {
                let started = Instant::now();
                while lock.load(Ordering::SeqCst) && started.elapsed() < OLD_TURN_EXIT_WAIT {
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
            }
            // The first queued message takes the lock back through the launch.
            let first = ctrl.inner.lock().unwrap().pending_messages.pop_front();
            if holding {
                ctrl.unlock_run();
            }
            let Some(first) = first else { return };
            adapter.materialize(&ctrl);
            let launched = match container {
                Some(c) => ctrl.spawn_container_turn(c.cm, c.container_name, c.base_cmd, first),
                None => adapter.launch(&ctrl, first),
            };
            if let Err(e) = launched {
                tracing::warn!(block_id = %ctrl.block_id, error = %e, "failed to start the replaced controller's queued turn");
                SubprocessController::publish_queued_spawn_error(&ctrl.block_id, &e, &ctrl.broker, &ctrl.filestore, &ctrl.mstore);
            }
        });
    }
}

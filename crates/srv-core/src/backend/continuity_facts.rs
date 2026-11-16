// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What AgentMux knows about an agent at the moment its session restarts, written
//! without a model (SPEC_AGENT_PREP_BEFORE_SESSION_RESTART_2026_11_14.md §6, item 4).
//!
//! The running summary is up to ten minutes old and needs an account that can
//! make it. These facts are read from the stores when the packet is built, so
//! "In flight" no longer depends on the summarizer alone: the Kanban tasks the
//! agent holds, the commands it left running, the status line it set and the
//! state of its working clone.

use crate::backend::obj::Block;
use crate::backend::storage::background_tasks::BackgroundTaskStatus;
use crate::backend::storage::store::Store;

/// Most entries listed per kind; the rest is a count.
const MAX_LISTED: usize = 5;
/// Longest a title, label or status line is written.
const MAX_TEXT_CHARS: usize = 120;

pub(crate) struct HeldTask {
    /// How the task is written (`#47659`, or `board#12`).
    pub reference: String,
    pub title: String,
}

pub(crate) struct Background {
    pub label: String,
    pub started_at_ms: i64,
    /// Whether the process was found alive just now; `None` when it has no pid to ask about.
    pub alive: Option<bool>,
}

pub(crate) struct StatusLine {
    pub text: String,
    pub state: Option<String>,
    pub set_at_ms: u64,
}

pub(crate) struct WorkingClone {
    pub branch: Option<String>,
    pub dirty_files: usize,
    pub checked_ms: Option<u64>,
}

#[derive(Default)]
pub(crate) struct Facts {
    pub tasks: Vec<HeldTask>,
    pub background: Vec<Background>,
    pub line: Option<StatusLine>,
    pub repo: Option<WorkingClone>,
}

fn short(text: &str) -> String {
    let one_line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if one_line.chars().count() <= MAX_TEXT_CHARS {
        return one_line;
    }
    let cut: String = one_line.chars().take(MAX_TEXT_CHARS - 1).collect();
    format!("{cut}…")
}

fn age(ms: i64) -> String {
    let s = (ms.max(0) / 1000) as u64;
    match s {
        0..=89 => format!("{s} s"),
        90..=5_399 => format!("{} min", (s + 30) / 60),
        _ => format!("{} h", (s + 1_800) / 3_600),
    }
}

/// The facts as the lines of the packet's block; empty when there is nothing to say.
pub(crate) fn render(facts: &Facts, now_ms: i64) -> String {
    let mut lines: Vec<String> = Vec::new();
    if !facts.tasks.is_empty() {
        let mut parts: Vec<String> =
            facts.tasks.iter().take(MAX_LISTED).map(|t| format!("{} {}", t.reference, short(&t.title))).collect();
        if facts.tasks.len() > MAX_LISTED {
            parts.push(format!("and {} more", facts.tasks.len() - MAX_LISTED));
        }
        lines.push(format!("- Kanban tasks you hold (claimed): {}. Re-read a task's brief with Tasks get (id), not from a summary.", parts.join("; ")));
    }
    if !facts.background.is_empty() {
        let mut parts: Vec<String> = facts
            .background
            .iter()
            .take(MAX_LISTED)
            .map(|b| {
                let state = match b.alive {
                    Some(true) => "process alive",
                    Some(false) => "process gone",
                    None => "no pid",
                };
                format!("{} (running {}, {state})", short(&b.label), age(now_ms - b.started_at_ms))
            })
            .collect();
        if facts.background.len() > MAX_LISTED {
            parts.push(format!("and {} more", facts.background.len() - MAX_LISTED));
        }
        lines.push(format!("- Background commands the record shows running: {}", parts.join("; ")));
    }
    if let Some(l) = &facts.line {
        let state = l.state.as_deref().map_or_else(String::new, |s| format!(" [{s}]"));
        let set = if l.set_at_ms > 0 { format!(", set {} ago", age(now_ms - l.set_at_ms as i64)) } else { String::new() };
        lines.push(format!("- Your status line{state}{set}: {}", short(&l.text)));
    }
    if let Some(c) = &facts.repo {
        let branch = c.branch.as_deref().map_or_else(|| "no branch".to_string(), |b| format!("branch {b}"));
        let checked = c.checked_ms.filter(|ms| *ms > 0).map_or_else(String::new, |ms| format!(" (checked {} ago)", age(now_ms - ms as i64)));
        lines.push(format!("- Working clone: {branch}, {} files with uncommitted changes{checked}", c.dirty_files));
    }
    lines.join("\n")
}

/// What a restart of the session would cost, in the words the log and the audit row use, or `None`
/// when nothing is lost: a Kanban task held, a command still running (one found dead is not), or
/// files with uncommitted changes in the working clone (SPEC_AGENT_PREP_BEFORE_SESSION_RESTART_2026_11_14.md §3.2).
pub(crate) fn something_lost(facts: &Facts) -> Option<&'static str> {
    if !facts.tasks.is_empty() {
        Some("holds a claimed task")
    } else if facts.background.iter().any(|b| b.alive != Some(false)) {
        Some("a background command is running")
    } else if facts.repo.as_ref().is_some_and(|c| c.dirty_files > 0) {
        Some("uncommitted changes in its working clone")
    } else {
        None
    }
}

/// [`something_lost`] for the agent in `block_id`, read from the stores. Blocking reads.
pub fn something_to_lose(mstore: &Store, identity: Option<&Store>, block_id: &str) -> Option<&'static str> {
    something_lost(&collect(mstore, identity, block_id))
}

/// The facts for the agent in `block_id`, from the stores. Blocking reads: call off the async workers.
pub(crate) fn collect(mstore: &Store, identity: Option<&Store>, block_id: &str) -> Facts {
    let mut facts = Facts::default();
    let Some(agent_id) = crate::identity::resolver::block_agent_id(mstore, block_id) else { return facts };
    let regs = crate::backend::reactive::get_global_handler().list_agents();
    let reg = regs.iter().find(|r| r.block_id == block_id);

    if let Some(identity) = identity {
        let uid = reg.and_then(|r| r.uid.clone()).unwrap_or_default();
        for item in identity.work_queue_claimed_by(&agent_id, &uid).unwrap_or_default() {
            if item.state != "claimed" {
                continue;
            }
            let reference = match identity.work_task_board(&item.id) {
                Ok(Some((board, num))) => board.task_ref(num),
                _ => "task".to_string(),
            };
            facts.tasks.push(HeldTask { reference, title: item.title });
        }
    }

    for task in mstore.background_task_list_for_block(block_id).unwrap_or_default() {
        if task.status != BackgroundTaskStatus::Running {
            continue;
        }
        let alive = task.pid.and_then(|p| u32::try_from(p).ok()).map(crate::backend::reactive::registry::pid_alive);
        facts.background.push(Background { label: task.label, started_at_ms: task.started_at_ms, alive });
    }

    let meta = mstore.get::<Block>(block_id).ok().flatten().map(|b| b.meta).unwrap_or_default();
    if let Some(d) = crate::backend::work_facts::live_phrase::declared_of(&meta) {
        facts.line = Some(StatusLine { text: d.text, state: d.state, set_at_ms: d.at_ms });
    }
    if let Some(reg) = reg {
        let channel = crate::backend::work_facts::cross_channel::own_channel();
        if let Some(w) = crate::backend::work_facts::collect_local(mstore, std::slice::from_ref(reg), &channel).pop() {
            if w.working_folder.is_some() && (w.branch.is_some() || w.git_checked_ms.is_some()) {
                facts.repo = Some(WorkingClone { branch: w.branch, dirty_files: w.dirty_count, checked_ms: w.git_checked_ms });
            }
        }
    }
    facts
}

#[cfg(test)]
#[path = "continuity_facts_tests.rs"]
mod tests;

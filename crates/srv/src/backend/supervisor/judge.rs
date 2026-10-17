// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The judge: one Haiku call through the ambient gateway that reads a
//! supervised agent's recent conversation and says whether to keep it going.
//! Spec §3.3.

use crate::ambient::call::{self, CliTarget};
use crate::ambient::purpose;
use crate::ambient::reply::{judge_line, FORMAT_RULES};
use crate::backend::obj::Block;
use crate::backend::storage::store::Store;

use super::{SupervisorMode, Verdict};

/// The longest reason or next step kept from a reply.
pub const TEXT_MAX_CHARS: usize = 300;

/// A verdict the judge gave, in the reply format.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Judgement {
    pub verdict: Verdict,
    pub reason: String,
    /// The suggested next step; Drive only, empty otherwise.
    pub next: String,
}

const ROLE: &str = "You supervise an AI coding agent that has just ended its turn. Read the end of its \
conversation and decide whether it should keep working on its own or stop for its user.";

const STEADY_RULES: &str = "Mode: steady. Say continue only when the agent stopped to check in (\"shall I \
continue?\", \"let me know if you want me to…\") or stopped short of work it had already planned or been asked \
to do, and nothing in what is left needs the user. Say needs_user when it asks a real question, offers a choice \
only the user can make, or faces an open question it cannot settle from the conversation.";

const DRIVE_RULES: &str = "Mode: drive. The agent should push forward on its own best judgement. Say continue \
when it stopped to check in, stopped short of its task, or stopped on an open question it can research or decide \
itself; give the best next step. Say needs_user only for something only the user can provide: a credential, a \
sign-in, a payment, or a product decision with no sensible default.";

const COMMON_RULES: &str = "In every mode:\n\
- done: the task is complete AND the conversation shows evidence (tests passing, a pull request opened or merged, \
the result shown). A bare claim of being done is not evidence: in drive say continue and ask it to show the \
evidence; in steady say needs_user.\n\
- A turn that ends by asking the user to approve, choose or decide something is never done, even when the user \
told the agent to ask: something is still waiting on an answer.\n\
- needs_user: the next step is risky, irreversible or outward-facing (deleting data, force-pushing, deploying, \
spending money, publishing, messaging people outside the team), even if the agent proposes it.\n\
- blocked: the agent cannot proceed (a failing tool it cannot fix, missing access) and says so.\n\
- When unsure between continue and needs_user, say needs_user.";

/// The prompt for one judgement.
pub fn build_prompt(mode: SupervisorMode, digest: &str, continuations: u32, limit: u32) -> String {
    let rules = match mode {
        SupervisorMode::Drive => DRIVE_RULES,
        _ => STEADY_RULES,
    };
    let next = match mode {
        SupervisorMode::Drive => "<next> is one sentence: the next step the agent should take, or - when the verdict is not continue.",
        _ => "<next> is a single -.",
    };
    let instruction = format!(
        "{ROLE}\n\n{rules}\n\n{COMMON_RULES}\n\nIt has been continued {continuations} times in a row (limit {limit}).\n\n\
         The answer text is: <verdict> | <reason> | <next>\n\
         <verdict> is one of continue, done, needs_user, blocked. <reason> is one sentence the user will read, \
         about this agent's work. {next}\n{FORMAT_RULES}"
    );
    crate::ambient::prompt::with_material(&instruction, "agent_conversation", digest)
}

/// Read an answer (the text after `ANSWER:`) as a judgement; `None` when it is
/// not in the form `<verdict> | <reason> | <next>`.
pub fn parse_answer(answer: &str, mode: SupervisorMode) -> Option<Judgement> {
    let mut parts = answer.splitn(3, '|').map(str::trim);
    let verdict = Verdict::parse(parts.next()?)?;
    let reason = clip(parts.next().unwrap_or(""));
    if reason.is_empty() {
        return None;
    }
    let next = clip(parts.next().unwrap_or(""));
    let next = if mode == SupervisorMode::Drive && verdict == Verdict::Continue && next != "-" { next } else { String::new() };
    Some(Judgement { verdict, reason, next })
}

/// One line, at most `TEXT_MAX_CHARS`.
fn clip(text: &str) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= TEXT_MAX_CHARS {
        return flat;
    }
    let mut out: String = flat.chars().take(TEXT_MAX_CHARS - 1).collect();
    out.push('…');
    out
}

/// What a judgement call came to.
#[derive(Debug)]
pub enum Outcome {
    Judged(Judgement),
    /// The CLI failed or timed out: worth one more try.
    Failed,
    /// The model answered, out of format: no verdict.
    NoVerdict,
    /// A newer request for this pane took over, or the call never ran.
    Superseded,
}

/// Ask the judge about `block_id`'s turn. `generation` must increase per pane.
pub async fn judge(
    mstore: &Store,
    block_id: &str,
    generation: u64,
    mode: SupervisorMode,
    digest: &str,
    continuations: u32,
) -> Outcome {
    let Some(mut slot) = call::admit(&purpose::SUPERVISOR_VERDICT, block_id, generation).await else {
        return Outcome::Superseded;
    };
    let Some(target) = mstore.get::<Block>(block_id).ok().flatten().and_then(|b| CliTarget::from_meta(&b.meta)) else {
        slot.abandon(crate::ambient::outcome::Outcome::Gated);
        return Outcome::NoVerdict;
    };
    let prompt = build_prompt(mode, digest, continuations, mode.limits().continuations);
    let reply = slot
        .run_held(&target, &prompt, |raw| {
            judge_line(raw, |answer| parse_answer(answer, mode).map(|_| answer.to_string()))
        })
        .await;
    if slot.is_superseded() {
        return Outcome::Superseded;
    }
    if reply.error.is_some() {
        return Outcome::Failed;
    }
    match parse_answer(&reply.text, mode) {
        Some(j) => Outcome::Judged(j),
        None => Outcome::NoVerdict,
    }
}

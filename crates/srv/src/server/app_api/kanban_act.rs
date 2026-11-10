// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The moves `kanban.act` makes, one method each. A method answers its reply, or the error that
//! refuses the move; `act_as` records that the board changed after any reply.

use super::*;

/// One `kanban.act` call: who makes it, what it asks and what the board holds.
#[derive(Clone, Copy)]
pub(super) struct Act<'a> {
    pub(super) state: &'a AppState,
    pub(super) req: &'a ActRequest,
    pub(super) actor: &'a Actor,
    /// The agents of this instance.
    pub(super) regs: &'a [AgentRegistration],
    /// The words that came with the move, cleaned.
    pub(super) text: &'a str,
    pub(super) now: i64,
}

/// How `release` / `reassign` ended: with the question that asks first, or with the task taken back.
pub(super) enum TakeBack {
    Asked(Value),
    Done(Value),
}

/// How a task is written (`#12`, or `work#12` off the default board).
fn task_ref(state: &AppState, id: &str) -> Option<String> {
    crate::server::work_tasks::ref_of(state, id).map(|(r, _)| r)
}

impl Act<'_> {
    /// The task the request names.
    pub(super) fn task(&self) -> Result<BoardTask, String> {
        let id = self.req.id.as_deref().ok_or("id is required")?;
        let id = resolve_id(self.state, id);
        let task = self.state.work_store.work_board_get(&id).map_err(|e| e.to_string())?.ok_or_else(|| "no such task".to_string())?;
        // An agent sees only the boards whose agent list has it; the operator sees them all.
        if !self.actor.is_operator {
            if let Some((board, _)) = self.state.work_store.work_task_board(&task.item.id).map_err(|e| e.to_string())? {
                if !board.visible_to(&self.actor.name, "") {
                    return Err("no such task".to_string());
                }
            }
        }
        Ok(task)
    }

    pub(super) fn create(&self) -> Result<Value, String> {
        let Act { state, req, regs, now, .. } = *self;
        let title = req.title.as_deref().map(str::trim).filter(|t| !t.is_empty()).ok_or("a title is required")?;
        let payload = req.payload.as_deref().map(str::trim).filter(|p| !p.is_empty()).unwrap_or(title);
        let after = resolve_after(state, req.after.as_deref().unwrap_or_default())?;
        Ok(match req.first.as_deref().map(str::trim).filter(|f| !f.is_empty()) {
            // "Spec first" (§5.1): a spec (or research) task, and a build
            // that starts after it, for the same agent.
            Some(first) => {
                if !crate::backend::kanban::is_research_kind(first) {
                    return Err("first: \"spec\" or \"research\"".into());
                }
                let first = first.to_ascii_lowercase();
                let (spec_id, spec_num) =
                    create_one(state, regs, req, &kanban::pair_title(&first, title), &kanban::spec_payload(&first, payload), &first, &after, now)?;
                let (build_id, build_num) = create_one(
                    state,
                    regs,
                    req,
                    &kanban::pair_title("build", title),
                    &kanban::build_payload(payload),
                    "build",
                    std::slice::from_ref(&spec_id),
                    now,
                )?;
                json!({
                    "ok": true, "id": spec_id, "num": spec_num, "ref": task_ref(state, &spec_id),
                    "build_id": build_id, "build_num": build_num, "build_ref": task_ref(state, &build_id),
                })
            }
            None => {
                let kind = req.kind.clone().unwrap_or_default();
                let (id, num) = create_one(state, regs, req, title, payload, &kind, &after, now)?;
                json!({ "ok": true, "id": id, "num": num, "ref": task_ref(state, &id) })
            }
        })
    }

    pub(super) fn edit(&self) -> Result<Value, String> {
        let Act { state, req, actor, now, .. } = *self;
        let store = &state.work_store;
        let t = self.task()?;
        let id = &t.item.id;
        // First: a refused list changes nothing.
        let after = req.after.as_ref().map(|given| resolve_after(state, given)).transpose()?;
        if let Some(after) = &after {
            if let Some(why) = store.work_deps_set(id, after).map_err(|e| e.to_string())? {
                return Err(why);
            }
        }
        store
            .work_task_edit(
                id,
                req.title.as_deref().map(clean).as_deref(),
                req.payload.as_deref().map(clean).as_deref(),
                req.kind.as_deref(),
                req.priority,
                now,
            )
            .map_err(|e| e.to_string())?;
        // Words an agent put in the payload are the agent's: a later start quotes them under its name
        // instead of delivering them as the operator's.
        let author = (!actor.is_operator && req.payload.is_some()).then_some(actor.name.as_str());
        store.work_card_edit(id, None, req.review, req.repo.as_deref(), author, now).map_err(|e| e.to_string())?;
        let mut body = String::new();
        if let Some(after) = after {
            body = after_words(store, &after);
        }
        record(state, id, &actor.name, actor.kind(), "edited", &t.item.state, &t.item.state, &body);
        Ok(json!({ "ok": true }))
    }

    /// Move the task to another board (`board`): a new task there with the next number, the original
    /// closed with a pointer. An agent sees only the boards its agent list has it on.
    fn move_to_board(&self, to: &str) -> Result<Value, String> {
        let Act { state, req, actor, now, .. } = *self;
        let t = self.task()?;
        let who = (!actor.is_operator).then_some((actor.name.as_str(), ""));
        crate::server::work_tasks::pick_board(state, Some(to), who, true)?;
        if let Some((from, _)) = state.work_store.work_task_board(&t.item.id).map_err(|e| e.to_string())? {
            if who.is_some_and(|(a, u)| !from.visible_to(a, u)) {
                return Err("no such task".into());
            }
        }
        let moved = state
            .work_store
            .work_task_move(&t.item.id, to, &actor.name, req.drop_deps == Some(true), now)
            .map_err(|e| e.to_string())?;
        Ok(json!({
            "ok": true, "id": moved.id, "ref": moved.to_ref, "from": moved.from_ref, "board": moved.board, "num": moved.num,
            "dropped_dependencies": moved.dropped_deps, "target_cleared": moved.target_cleared,
        }))
    }

    pub(super) fn move_task(&self) -> Result<Value, String> {
        let Act { state, req, actor, regs, now, .. } = *self;
        if let Some(to) = req.board.as_deref().map(str::trim).filter(|b| !b.is_empty()) {
            return self.move_to_board(to);
        }
        let store = &state.work_store;
        let t = self.task()?;
        let id = &t.item.id;
        if let Some(agent) = req.agent.as_deref() {
            let (name, uid) = target_of(state, regs, agent);
            if !store.work_assign(id, &name, &uid, now).map_err(|e| e.to_string())? {
                return Err("only a waiting task can be moved to another agent".into());
            }
            let to = if name.is_empty() { "anyone".to_string() } else { name };
            record(state, id, &actor.name, actor.kind(), "assigned", &t.item.state, &t.item.state, &format!("for {to}"));
        }
        match req.hold {
            Some(true) => {
                if let Some(m) = store.work_hold(id, now).map_err(|e| e.to_string())? {
                    record(state, id, &actor.name, actor.kind(), "held", &m.from, &m.to, "");
                }
            }
            Some(false) => {
                if let Some(m) = store.work_unhold(id, now).map_err(|e| e.to_string())? {
                    record(state, id, &actor.name, actor.kind(), "unheld", &m.from, &m.to, "");
                }
            }
            None => {}
        }
        if let Some(rank) = req.rank {
            store.work_card_edit(id, Some(rank), None, None, None, now).map_err(|e| e.to_string())?;
        }
        Ok(json!({ "ok": true }))
    }

    pub(super) async fn start(&self) -> Result<Value, String> {
        let Act { state, req, actor, regs, now, .. } = *self;
        let store = &state.work_store;
        let t = self.task()?;
        let agent = req.agent.as_deref().ok_or("start: which agent?")?;
        let reg = agent_named(regs, agent).ok_or_else(|| format!("no agent named {agent} here"))?;
        if !alive(&reg) {
            return Err(format!("{} isn't running", reg.agent_id));
        }
        let uid = reg.uid.clone().unwrap_or_default();
        let started = store
            .work_start_for(&t.item.id, &reg.agent_id, &uid, now, LEASE_MS)
            .map_err(|e| e.to_string())?
            .ok_or("only a waiting task can be started")?;
        let fresh = store.work_board_get(&t.item.id).map_err(|e| e.to_string())?.ok_or("no such task")?;
        if let Err(e) = tell(state, &reg, actor, start_text(&fresh, started.attempts, actor)).await {
            // Not delivered: put it back as it was, its attempt not used,
            // rather than leave it held by an agent that never heard of it.
            let _ = store.work_unstart(
                &t.item.id,
                &reg.agent_id,
                started.attempts,
                &t.item.state,
                &t.item.target_agent,
                &t.item.target_agent_uid,
                now,
            );
            record(state, &t.item.id, &actor.name, actor.kind(), "start_failed", work_state::CLAIMED, &t.item.state, &format!("couldn't reach {}", reg.agent_id));
            changed(state);
            return Err(format!("couldn't reach {}: {e}", reg.agent_id));
        }
        record(state, &t.item.id, &actor.name, actor.kind(), "started", &t.item.state, work_state::CLAIMED, &format!("on {}", reg.agent_id));
        Ok(json!({ "ok": true, "attempt": started.attempts }))
    }

    pub(super) async fn answer(&self) -> Result<Value, String> {
        let Act { state, actor, regs, text, now, .. } = *self;
        let store = &state.work_store;
        let t = self.task()?;
        if text.is_empty() {
            return Err("the answer is empty".into());
        }
        if t.item.state != work_state::BLOCKED {
            return Err("this task isn't waiting on a question".into());
        }
        let reg = holder_of(regs, &t.item).cloned().filter(alive).ok_or("its agent isn't running")?;
        let msg = format!(
            "Answer{} on task {} \"{}\": {text}\n\nCarry on with it (id {}, attempt {}).",
            if actor.is_operator { String::new() } else { format!(" from {}", quote_in_system_note(&actor.name)) },
            t.card.name(),
            quote_in_system_note(&t.item.title),
            t.card.id_arg(),
            t.item.attempts
        );
        tell(state, &reg, actor, msg).await.map_err(|e| format!("couldn't reach {}: {e}", reg.agent_id))?;
        store.work_answered(&t.item.id, now, LEASE_MS).map_err(|e| e.to_string())?;
        store.work_card_set_question(&t.item.id, "", now).map_err(|e| e.to_string())?;
        record(state, &t.item.id, &actor.name, actor.kind(), "answered", work_state::BLOCKED, work_state::CLAIMED, text);
        Ok(json!({ "ok": true }))
    }

    // Take back a leader's acceptance (the spec's D5): Done to Review, for a week. What
    // the acceptance set off (a started build, a merged PR) stays as it is.
    pub(super) fn reopen(&self) -> Result<Value, String> {
        let Act { state, actor, text, now, .. } = *self;
        let store = &state.work_store;
        let t = self.task()?;
        let accepts = store.work_delegate_accepts(now - crate::backend::kanban_delegate::REOPEN_WINDOW_MS).map_err(|e| e.to_string())?;
        if !accepts.contains_key(&t.item.id) {
            return Err("only a task an agent accepted, in the last 7 days, can be reopened".into());
        }
        let m = store.work_reopen(&t.item.id, now).map_err(|e| e.to_string())?.ok_or("only a done task can be reopened")?;
        record(state, &t.item.id, &actor.name, actor.kind(), "reopened", &m.from, &m.to, text);
        Ok(json!({ "ok": true }))
    }

    pub(super) fn cancel(&self) -> Result<Value, String> {
        let Act { state, req, actor, text, now, .. } = *self;
        let store = &state.work_store;
        let t = self.task()?;
        // A cancelled spec approves nothing: what waits on it is
        // cancelled too, or put on hold for the operator (§6). A
        // cancelled task counts as finished for what starts after it, so
        // its waiting builds go on hold *before* it is cancelled: until
        // then the unfinished spec keeps them from being claimed, and no
        // claim can slip in between.
        let research = crate::backend::kanban::is_research_kind(&t.item.kind);
        let why = format!("its {} {} was cancelled", t.item.kind.trim().to_ascii_lowercase(), t.card.name());
        let mut held: Vec<String> = Vec::new();
        let mut were_held: Vec<String> = Vec::new();
        if research {
            for f in store.work_followers_of(&t.item.id).map_err(|e| e.to_string())? {
                match f.state.as_str() {
                    work_state::OPEN => {
                        if store.work_hold(&f.id, now).map_err(|e| e.to_string())?.is_some() {
                            held.push(f.id);
                        }
                    }
                    work_state::HELD => were_held.push(f.id),
                    _ => {}
                }
            }
        }
        if !store.work_queue_cancel(&t.item.id, text, now).map_err(|e| e.to_string())? {
            // Nothing to cancel: let the builds go as they were.
            for id in &held {
                let _ = store.work_unhold(id, now);
            }
            return Err("it is already finished".into());
        }
        record(state, &t.item.id, &actor.name, actor.kind(), "cancelled", &t.item.state, work_state::CANCELLED, text);
        let mut followers = 0;
        if req.followers.as_deref() == Some("hold") {
            for id in &held {
                record(state, id, &actor.name, actor.kind(), "held", work_state::OPEN, work_state::HELD, &why);
                followers += 1;
            }
        } else {
            for id in held.iter().chain(were_held.iter()) {
                if store.work_queue_cancel(id, &why, now).map_err(|e| e.to_string())? {
                    record(state, id, &actor.name, actor.kind(), "cancelled", "", work_state::CANCELLED, &why);
                    followers += 1;
                }
            }
        }
        Ok(json!({ "ok": true, "followers": followers }))
    }

    pub(super) fn retry(&self) -> Result<Value, String> {
        let Act { state, actor, now, .. } = *self;
        let store = &state.work_store;
        let t = self.task()?;
        let m = store.work_retry(&t.item.id, now).map_err(|e| e.to_string())?.ok_or("only a failed task can be retried")?;
        record(state, &t.item.id, &actor.name, actor.kind(), "retried", &m.from, &m.to, "");
        Ok(json!({ "ok": true }))
    }

    pub(super) async fn track(&self) -> Result<Value, String> {
        let Act { state, req, regs, now, .. } = *self;
        let store = &state.work_store;
        let block = req.block_id.as_deref().ok_or("track: which agent pane?")?;
        let reg = regs.iter().find(|r| r.block_id == block).cloned().ok_or("that pane has no agent")?;
        let facts = {
            let mstore = state.mstore.clone();
            let r = reg.clone();
            let channel = crate::backend::work_facts::cross_channel::own_channel();
            tokio::task::spawn_blocking(move || crate::backend::work_facts::collect_local(&mstore, &[r], &channel))
                .await
                .unwrap_or_default()
                .into_iter()
                .next()
        };
        let title = req
            .title
            .as_deref()
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .map(str::to_string)
            .or_else(|| facts.as_ref().and_then(|f| f.goal.clone()))
            .unwrap_or_else(|| format!("{}'s current work", reg.agent_id));
        let title: String = clean(&title).chars().take(200).collect();
        let uid = reg.uid.clone().unwrap_or_default();
        let item = WorkItem {
            id: Uuid::new_v4().to_string(),
            title: title.clone(),
            payload: title.clone(),
            kind: String::new(),
            target_agent: reg.agent_id.clone(),
            target_group: String::new(),
            priority: 0,
            state: String::new(),
            claimed_by: String::new(),
            target_agent_uid: uid.clone(),
            claimed_by_uid: String::new(),
            claim_expires: None,
            attempts: 0,
            max_attempts: 3,
            created_by: OPERATOR.into(),
            created_by_uid: String::new(),
            created_at: now,
            updated_at: now,
            not_before: None,
            result: String::new(),
        };
        store.work_queue_enqueue(&item).map_err(|e| e.to_string())?;
        let card = NewCard {
            board: String::new(),
            repo: facts.as_ref().and_then(|f| f.repo_key()).unwrap_or_default(),
            branch: facts.as_ref().and_then(|f| f.branch.clone()).unwrap_or_default(),
            review_required: true,
            requested_by: OPERATOR.into(),
            requested_by_kind: requested::OPERATOR.into(),
            request_text: String::new(),
            filed_by: String::new(),
        };
        let num = store.work_card_create(&item.id, &card, now).map_err(|e| e.to_string())?;
        let started = store
            .work_start_for(&item.id, &reg.agent_id, &uid, now, LEASE_MS)
            .map_err(|e| e.to_string())?
            .ok_or("couldn't take it for the agent")?;
        record(state, &item.id, OPERATOR, OPERATOR, "tracked", "", work_state::CLAIMED, &format!("{}'s work", reg.agent_id));
        let note = format!(
            "[AgentMux] The operator is following your current work in the Kanban as task #{num} \"{}\". \
             When it's done, hand it in with Tasks submit (id {num}, attempt {}).",
            quote_in_system_note(&title),
            started.attempts
        );
        let target = if uid.is_empty() { reg.agent_id.clone() } else { uid };
        // Informational: a session gone cold gets it with its next turn rather than waking.
        let extra = state.config_watcher.get_settings().extra;
        if let Err(e) = crate::backend::cold_hold::note_or_wait_if_cold(&state.mstore, &extra, &reg.block_id, &target, &format!("track:{num}"), &note) {
            tracing::info!(error = %e, "kanban: track note not delivered");
        }
        Ok(json!({ "ok": true, "num": num }))
    }

    pub(super) async fn take_back(&self) -> Result<TakeBack, String> {
        let Act { state, req, actor, regs, text, now } = *self;
        let store = &state.work_store;
        let t = self.task()?;
        if !matches!(t.item.state.as_str(), work_state::CLAIMED | work_state::BLOCKED) {
            return Err("only a task an agent holds can be taken back".into());
        }
        let reassign = req.action == "reassign";
        // `reassign` names who it goes to: an agent (running here or not), or
        // anyone when empty. `release` leaves who it is for as it was.
        let retarget = if reassign {
            let agent = req.agent.as_deref().ok_or("reassign: to which agent? (empty for anyone)")?.trim();
            Some(target_of(state, regs, agent))
        } else {
            None
        };
        // Work the holder did lives on its branch or PR; taking the task back
        // doesn't move it. The operator confirms having read that.
        let links: Vec<String> = [("a branch", &t.card.branch), ("a PR", &t.card.pr_url)]
            .iter()
            .filter(|(_, v)| !v.trim().is_empty())
            .map(|(what, v)| format!("{what} ({})", v.trim()))
            .collect();
        if !links.is_empty() && req.confirm != Some(true) {
            return Ok(TakeBack::Asked(json!({
                "ok": false,
                "confirm": format!(
                    "It has {}. That work stays where it is, and whoever gets the task has to pick it up from there. Take it back anyway?",
                    links.join(" and ")
                ),
            })));
        }
        let holder = if t.item.claimed_by.is_empty() { "its agent".to_string() } else { t.item.claimed_by.clone() };
        let reason = if text.is_empty() {
            format!("taken back from {holder}")
        } else {
            format!("taken back from {holder}: {text}")
        };
        let back = store
            .work_take_back(&t.item.id, retarget.as_ref().map(|(a, u)| (a.as_str(), u.as_str())), &reason, now)
            .map_err(|e| e.to_string())?
            .ok_or("it moved meanwhile")?;
        let to = match &retarget {
            Some((name, _)) if !name.is_empty() => format!("; now for {name}"),
            Some(_) => "; now for anyone".to_string(),
            None => String::new(),
        };
        let kind = if reassign { "reassigned" } else { "taken_back" };
        record(state, &t.item.id, &actor.name, actor.kind(), kind, &t.item.state, work_state::OPEN, &format!("{reason}{to}"));
        // Cancel tells nobody, so the old holder is told here: it may still be
        // working on the task, or be about to hand it in.
        let note = format!(
            "[AgentMux] {} took task {} \"{}\" back from you. Stop working on it and don't hand it in: \
             your claim (attempt {}) is closed.{}",
            actor.who(),
            t.card.name(),
            quote_in_system_note(&t.item.title),
            t.item.attempts,
            if text.is_empty() { String::new() } else { format!(" Reason given: {}", quote_in_system_note(text)) }
        );
        let target = if t.item.claimed_by_uid.is_empty() { t.item.claimed_by.clone() } else { t.item.claimed_by_uid.clone() };
        if !target.is_empty() {
            if let Err(e) = crate::backend::reactive::get_global_handler().deliver_system_note(&target, &note) {
                tracing::info!(error = %e, "kanban: take-back note not delivered");
            }
        }
        Ok(TakeBack::Done(json!({ "ok": true, "id": back.id })))
    }

    pub(super) fn media_remove(&self) -> Result<Value, String> {
        let Act { state, req, actor, text, now, .. } = *self;
        let store = &state.work_store;
        let t = self.task()?;
        let media_id = req.media_id.ok_or("media_remove: media_id is required")?;
        let gone = store
            .work_media_remove(&t.item.id, media_id, &actor.name, now)
            .map_err(|e| e.to_string())?
            .ok_or("no such file on this task, or it is already removed")?;
        // The bytes go when no task names them.
        if !store.work_media_named(&gone.sha256).unwrap_or(true) {
            if let Some(root) = crate::backend::task_media::default_root() {
                crate::backend::task_media::remove_content(&root, &gone.sha256, &gone.mime);
            }
        }
        record(state, &t.item.id, &actor.name, actor.kind(), "media_removed", &gone.name, "", text);
        Ok(json!({ "ok": true }))
    }

    pub(super) fn ui_change(&self) -> Result<Value, String> {
        let Act { state, req, actor, text, now, .. } = *self;
        let store = &state.work_store;
        let t = self.task()?;
        let flag = req.ui_change.ok_or("ui_change: true or false is required")?;
        store.work_ui_set(&t.item.id, Some(flag), None, 0, now).map_err(|e| e.to_string())?;
        record(state, &t.item.id, &actor.name, actor.kind(), "ui_change", if flag { "yes" } else { "no" }, "", text);
        Ok(json!({ "ok": true }))
    }
}

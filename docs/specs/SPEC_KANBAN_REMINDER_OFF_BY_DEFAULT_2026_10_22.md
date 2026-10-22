# Spec: the Kanban's backlog reminder off by default, with a setting to turn it on

**Status:** active — implemented in #5322.
**Date:** 2026-10-22
**Author:** AgentA@Area54, at the operator's request
**Verified against:** `agentmux` `main` @ `cc46a8c21` (2026-10-22), by reading code, and live on 0.59.38.
**Changes:** `SPEC_WARDEN_KANBAN_2026_10_20.md` §6.5, "The reminder" (its default only).

## 1. Ask

The operator, 2026-10-22, after an agent waiting on the operator's decision received the same reminder six times in a few hours: "lets write a spec that disables the kanban notice on default, and adds a setting to enable/disable it".

## 2. Today

- **What it is.** srv's reminder loop (`crates/srv/src/backend/kanban.rs`, `run_reminder_loop`) looks every 30 s. An agent that has been idle for 2 minutes (`IDLE_MS`) with tasks waiting in its backlog gets one system note, once per idle period: `[AgentMux] You have 16 tasks waiting in your backlog in the Kanban: #6 "…", … Take the next with Tasks claim when you're ready.`
- **Default.** On. `reminders_enabled` reads the setting `kanban:reminders` and treats a missing value as `true`. A test pins that (`reminders_are_on_unless_turned_off`).
- **Turning it off.**
  - For everyone: write `"kanban:reminders": false` into `settings.json` by hand. The setting is in neither `schema/settings.json` nor the Settings pane.
  - For one agent: set its block's `kanban:continue` to `false`. There is no UI for that either. It also stops the agent being offered its next task when it finishes one (`crates/srv/src/server/work_tasks.rs`).
- **Why it misfires.** "Idle with tasks waiting" includes an agent that is idle because it is waiting on the operator: a question asked, a decision it can't make, an instruction not to start the backlog yet. The note repeats each time the agent finishes a turn and goes idle again. Every reply to it is a turn spent saying "still waiting". On 2026-10-21–22 one agent received it six times while holding 16 tasks the operator had said not to start.

## 3. Design

### 3.1 Off unless turned on

- `kanban:reminders` defaults to `false`: `reminders_enabled` returns `true` only for an explicit `true`.
- Existing installs need no migration. Nobody can have turned it on, since on was the only state, and a stored `false` stays off.
- The test becomes `reminders_are_off_unless_turned_on`: missing → off, `false` → off, `true` → on.

### 3.2 The setting

- **Schema.** `schema/settings.json` gains:

  ```json
  "kanban:reminders": {
    "type": "boolean",
    "default": false,
    "description": "Remind an idle agent of the tasks waiting in its Kanban backlog. Off by default."
  }
  ```

- **Settings pane.** A toggle row like "Confirm before closing a tab" (`sections/window-panes-section.tsx`): a `ToggleControl` bound to `kanban:reminders`.
  - **Label:** "Remind idle agents of their backlog".
  - **Description:** "An agent with tasks waiting in its Kanban backlog gets a note after 2 minutes idle, once each time it goes idle."
  - **Where:** in the Advanced section (`sections/advanced-section.tsx`), since Settings has no Agents section.
  - **Search keywords:** "kanban", "backlog", "reminder", "kanban:reminders". With these it appears in the settings search (`settings-index.ts`).
- **The Kanban.** The board's toolbar (Warden › Kanban, `warden-kanban/warden-kanban-manager.tsx`) gets a third switch, "Reminders", beside "Lanes" and "Untracked", titled with the Settings description. The operator decides about reminders while looking at the backlog, and the two places write the same setting.
- **Live.** srv reads the setting on every look, so a change applies within 30 s without a restart. The idle clocks reset when it is off (`idle_since.clear()`), so turning it on never fires a burst of overdue notes.

### 3.3 What doesn't change

- **Finishing a task still offers the next one.** That isn't a notice: `submit` and `complete` answer the agent's own call with its next waiting task, and the Tasks guidance agents get already says to take it. It still follows `kanban:continue`.
- **`kanban:continue`** still turns one agent's going-on off, both the next-task answer and, with reminders on, the reminder.
- **The reminder itself**, when on: timing, text, once per idle period, never for held or moved tasks.

### 3.4 Docs

- `SPEC_WARDEN_KANBAN_2026_10_20.md` §6.5's last sentence says the reminder is on unless `kanban:reminders: false`. It changes to "off unless the operator turns it on in Settings (`kanban:reminders`)", citing this spec.
- The `kanban.rs` module comment and the `SETTING_REMINDERS` doc ("default on") change the same way.
- The public docs page for the Kanban, if it describes the reminder, follows in agentmux-docs.

## 4. Tests

- **srv:** `reminders_enabled` for missing / `false` / `true`.
- **Settings:**
  - the row renders unchecked with no stored value, and checked for `true`;
  - toggling writes `kanban:reminders`;
  - the settings search finds it by "kanban" and "reminder".
- **Kanban:** the toolbar's "Reminders" switch shows the same value as the Settings row and writes the same key.
- **By hand, in a build:**
  - with reminders off by default, an idle agent with backlog tasks gets no note after 2 minutes;
  - turned on, it gets one within about 2.5 minutes, and not again until it has worked and gone idle again;
  - turned off again, nothing further arrives.

## 5. Plan

One PR:
- the default in `kanban.rs` and its test;
- the schema entry;
- the Settings row and its search entry;
- the Kanban toolbar switch;
- the doc edits in §3.4;
- a changeset saying reminders are now off by default and where to turn them on.

## 6. Open questions

1. **A per-agent opt-in.** Should an operator be able to turn reminders on for one agent while the setting is off, the reverse of `kanban:continue`? Not needed for the ask; it would be a block-meta switch on the agent's Kanban lane.
2. **"Waiting on you" agents.** With reminders on, an agent whose last turn asked the operator something still gets reminded. srv already knows a pending question when the agent used `Tasks` `ask`, but not one asked in plain text. Should the reminder skip an agent whose last reply ended with a question to the operator? It would be a later refinement, not part of this change.

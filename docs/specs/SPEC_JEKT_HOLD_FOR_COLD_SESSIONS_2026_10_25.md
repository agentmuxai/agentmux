# Spec: hold a routine message for an agent whose session has gone cold

**Status:** active — approved by the operator ("proceed with recommendations") and by the platform lead for the rules in §3 and the choice of default-hold in §4. Built in Kanban #2971.
**Date:** 2026-10-25
**Author:** Naki-2@narko
**Verified against:** `agentmux` `main` @ `48328d93e` (2026-10-25), by reading code; the build was tested against it.
**Changes:** `SPEC_DURABLE_JEKT_DELIVERY_2026_09_24.md` (a new reason to hold), `SPEC_UNRESPONSIVE_AGENTS_2026_10_23.md` §5 (the hold path it shares).

## 1. Ask

A Claude session keeps its context in the provider's prompt cache for about an hour. A message that reaches an agent after it has been quiet longer than that starts a turn that pays for the whole context again. Many of those messages are acknowledgements, thanks and status lines that could have waited until the agent next had a reason to run. This spec holds those, and delivers them the next time the agent is awake anyway.

## 2. Summary

- A message is **held** when its target is a running Claude session whose last output was more than 55 minutes ago (the setting `jekt:cold_hold_minutes`, 0 turns the hold off), and the message is routine and not urgent (§3).
- The sender is told at once: `HELD for X (idle)`, with how to wake the agent (`urgent: true`).
- A held message is delivered, oldest first and with the usual "(sent …, delivered …)" prefix, as soon as the agent writes anything again (the operator types, an urgent message arrives, a schedule fires), or after 24 hours.

## 3. Which messages are held

A message is held only if **all** of these are true. Every doubt delivers it as before.

| Rule | Why |
|---|---|
| Host tier, full key, with a message id, not a `cron` fire, and not sent by the agent to itself | The set the durable hold already takes. WAN, LAN and channel messages are never held, so review and CI results on an agent's own PR still wake it |
| The sender is proven (`sig_verified`), the tier is not `sensitive`, it is not a transcript request, it carries no files, and the text raises no sensitive flag | The receiver's rules treat those differently; a held row cannot keep every verdict |
| `PRIORITY` is not `urgent` or `high` | The sender asked for it now |
| It names no task (`#N`) that is open, on hold, claimed or blocked and targeted at, or held by, the target | A message that hands out work must not stall |
| The sender did not file a task for the target in the last 15 minutes | The usual order is "file the task, then message the agent" |
| The target is a Claude session with a controller, no turn in flight, and output more than the limit ago | Another provider's cache works differently; a session that has never written has nothing to lose |
| The target has fewer than 64 held messages | Beyond the cap, deliver |

Swarm broadcasts and the operator's own words never go through this path.

## 4. The sender's side

- **Answer:** `success: false, held: true, held_reason: "idle"`. The `Message` tool says `HELD for X (idle)`, that the message waits until X is next active, and that sending it again with `urgent: true` wakes X now.
- **`urgent: true`** on `Message` `send` and `broadcast` sets `PRIORITY=urgent`. An urgent resend drops the held copy with the same sender and text, so the agent does not get it twice.
- **Broadcast:** a held target is listed under `held`, apart from `failed`.
- **Guidance:** the operator-config Messaging line says to pass `urgent: true` or name the task's `#` when assigning or unblocking work.

## 5. Delivery

- **Storage:** the durable hold's table (`db_jekt_held`) gains a `hold_reason` column (`idle` for these rows). No schema bump: an older build ignores the column and delivers the row at once.
- **Replay:** the 30-second pass skips an `idle` row while the target is still cold and delivers it when the target is warm. The pass already delivers oldest first, eight a pass, and the first delivery starts the turn the rest join.
- **Expiry:** at 24 hours an `idle` row is delivered, not dropped, so one quiet agent still gets everything once. It is dropped a whole TTL later only if its agent never came back.
- **Replay keeps the original verdicts and send time**, as for the other held rows.

## 6. Files

- `backend/cold_hold.rs`: the rules, the setting, the task-number reader.
- `server/jekt_cold.rs`: the decision and the hold; `backend/cold_hold.rs` also holds the note helper (§8); `server/reactive.rs` calls it after the quota check.
- `server/jekt_held.rs`, `backend/storage/jekt_held.rs`, `migrations.rs`: the reason column, the ready query, expiry.
- `storage/work_board.rs`: the two task lookups.
- `crates/mcp`: `urgent` on `Message`, the `idle` answer, broadcast's `held`.
- `crates/common/api_types.rs`: `priority` on the request.

## 7. Tests

- The rules: setting values, cold and warm, a clock step back, who may wait.
- The decision: held cold, delivered warm; urgent wakes and drops its copy; a named task and a just-filed task wake it.
- The store: the ready query, expiry and the resend cleanup.
- End to end through the inject route: held, left alone while cold, delivered once warm, urgent and sensitive messages not held.
- The MCP answer and `urgent` flag.

## 8. Notes that wait

Two of srv's own `[AgentMux]` notes are informational and now wait for a cold session instead of waking it (a warm session gets them at once, as before). They are kept under a key and go with the agent's next message, after its text, as the held notes already do (`reactive::held_notes`); they never start a turn.

| Note | Key | Why it can wait |
|---|---|---|
| "The operator is following your current work as task #N" (Kanban track) | `track:<num>` | It tells the agent to hand the task in when done; nothing is lost if that is a turn later |
| "Your worker X finished a turn without reporting to you" | `worker:<worker id>` (a newer one replaces an older) | A status line for a leader; the worker's own failure or stop, and a worker waiting on an answer, still wake the leader |

Everything else that wakes today still does: reminders, "build unlocked", the wake at a quota reset, a worker that failed or stopped or is waiting for an answer, job incident notes, account notices, the operator taking a task back, and the replies to upgrade and desktop requests.

## 9. Out of v1

| What | Why not now |
|---|---|
| A control for the setting in Settings | It is a plain key in the settings file |
| Holding WAN and LAN messages | A held row would lose the proof it arrived with, and review and CI notices should wake the agent |
| Other providers | Their caches differ; measure first |

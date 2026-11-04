# SPEC: an agent retakes a browser pane, and takes one it did not open (2026-11-04)

**Status:** active — approved by the operator (2026-11-04, via AgentY); built in #18112: retake, Give back and `handoff_status` first (what shipped is marked below); the idle rule, `take_control`, `release` and `driver` in the layout follow
**Author:** Camper-2@narko, 2026-11-04, at the operator's direction

Task #18111; the build is #18112. Author: Camper-2.
The operator's words (2026-11-01, said to Applo, relayed by AgentY): "agents need to be able to retakeover a browser pane after the user has taken it over. it can also commandeer browser panes."
Source: Applo's field notes (`REPORT_BROWSER_DRIVING_FIELD_NOTES_2026_10_30.md`, in Applo's workspace), items 9, 10 and 8's `handoff_status`.
The operator green-lit this spec's recommendations (2026-11-04, via AgentY): every question below is answered, none is left open. Nothing here costs money or needs a new external account.

## Recommendation in one paragraph

Control of a browser pane is always visible and always one click from the person. An agent can **retake** a pane it lost (it handed it off, or the person took it over) once the person gives it back or has been idle in it for a minute; it can **take control** of a pane it did not open when it could have opened that pane's profile, and it must ask the person in the pane when the pane shows their Personal sign-ins. Neither is silent: the pane header says who drives, a banner says when that changed, and the person takes it back with the existing Take over click. Nothing is an operator-only gate: the person's say is asked in the pane where their own sign-ins or hands are involved, as a handoff already does.

## How it works today (main, 2026-11-04)

- Ownership: `browser_owner::record` keeps block -> agent in memory and mirrors it to the block meta `browser:owner_agent`. Every agent call goes through `target_for` -> `browser_owner::check`, which refuses a pane the agent does not own ("not a browser pane you own").
- The person's takeover clears that meta key. `guard_client_meta_write` then calls `forget_because(Lost::TookOver)`, which also ends any auto-approve grant, and the agent's next call is refused with "the user took over pane X (N minutes ago)". The record lasts about an hour (256 panes).
- Handoff (`/api/v1/ui/browser/handoff`): the pane shows the reason with Done/Cancel, the agent's call blocks for up to the timeout (15 minutes by default), and answers `done`, `cancelled` or `timed_out`. If the agent's session ends in between, nothing records the outcome ("outcome unknown").
- Per-pane pause and "Pause all agent browsing" are the person's, and the agent cannot lift them.
- `Pane layout` lists browser panes without saying who drives them; the header badge ("Driven by X") reads the meta key.
- Profiles: a pane's `browser:identity` is fixed at open (#17135 gives every agent its own profile and `check_agent_may_use`, which this spec reuses).

## Decisions

### 1. Retake (a pane the agent lost)

1. **`Browser retake`** with `pane`. It makes the agent the pane's driver again on the same pane, session and page: no reopen, no new profile. The answer says where the page is now (the address and title the act answers already carry) and to take a snapshot first, because the person may have moved on.
2. **When it works.** The agent lost the pane (a record of `TookOver` for it, kept 24 hours) and one of: the person pressed **Give back to X** in the pane header; or the person took the pane over while the agent's own hand-off was up, which that takeover ended; or the person has not touched the pane (no input events) for 60 seconds. Otherwise it is refused with what to do: "the user took over pane P (4 min ago) and is using it: wait until they press Give back, or ask them in chat". No force flag: if the person is in it, they decide. *Built first: Give back, and the takeover during a hand-off. The 60-second idle rule needs the host to report the person's last input in a pane, and follows in its own PR; until then a person who walked away without pressing Give back is asked in chat.*
3. **The pane shows control.** The header says "Driven by X", or, after a takeover, "You are driving this pane" with a **Give back to X** button. srv writes the block meta `browser:last_driver` (the agent's id) whenever an agent becomes a pane's driver (open, popup, retake) and keeps it through the takeover. Give back is the person's click, relayed by the host like every answer to a pane's banner (`browser_attention_resolve` with the decision `give_back`), so no agent can give itself the pane; srv then writes `browser:given_back` (X). Neither key can be written by a client. That lets X retake; X is told in its next refusal. A retake brings the pane's site limit back (the header's `browser:allowed_origins` outlives the Take over, which ended the limit in srv). (A "retook control" notice for 10 seconds is not built.) The person's Take over click is unchanged and always wins.
4. **What a retake does not restore:** an auto-approve grant ended at the takeover stays ended; the agent asks again (`request_auto_approve`). A pause stays until the person lifts it.
5. **`handoff_status`.** A handoff's answer carries its `handoff_id`, which is also the `id` in the pane's banner meta. `Browser handoff_status` with `handoff_id` (or `pane`, for the newest) answers `pending`, `done`, `cancelled`, `timed_out` or `abandoned`, with the times, kept 24 hours in memory and shown only to the agent that asked for the handoff. `abandoned` means the agent's call ended before the person answered: the request's banner goes down with the call, so nobody can answer it, and the agent makes a new hand-off if it still needs the person. (Waiting again on a pending hand-off with `wait_seconds` is not built: the hand-off lives in the call that asked for it.)
6. **Restart.** `browser:last_driver` and `browser:given_back` are in the block meta, so after an srv restart the pane still offers Give back, and a Give back (before or after the restart) still lets X retake. The one thing lost is "the person took over during X's hand-off", which lives in memory.

### 2. Take control (a pane the agent did not open)

1. **`Browser take_control`** with `pane` (any browser pane id from `Pane layout`, which now shows each browser pane's `driver`: an agent, `person`, or none). Optionally `allowed_origins`, to limit it as `open` does.
2. **The rule is AgentX's:** an agent may take a pane only if it could have opened that pane's profile. So: the agent's own profile, a profile the user lets agents use, and Incognito are fine; the user's Personal pane is allowed for a native agent but asks (see 3); an unshared user profile or another agent's profile is refused with "the user hasn't let agents use <profile>: switch on Agents may use it in Settings → Browser". A container agent never takes a Personal or unshared pane, with no prompt, and the refusal says what to do (ask the person to share the profile).
3. **When the person is asked.** A pane that shows the user's Personal profile, or that the person drove in the last 60 seconds, shows a banner in the pane: "X wants to drive this pane. Allow / Not now" for 2 minutes. The call waits for the answer, as a handoff does; the answer is `allowed`, `declined` or `timed_out`. It is the person's own pane and sign-ins being used, so they are asked where they are looking; it is not an operator gate and any agent can ask.
4. **A pane another agent drives.** Refused with the owner's name, unless that agent runs the new `Browser release` (it hands its pane over: the next agent's `take_control` then works without a prompt), or the person says yes to the prompt in 3 (it applies here too). This keeps two agents from fighting over a pane.
5. **Visible and reversible.** The header shows "Driven by X (took control)" with the existing Take over button; the pane's Log records "X took control (profile P, the person last acted N s ago)". A person's takeover ends it as for any pane and starts the retake path of §1.
6. **Limits on a pane taken, not opened.** Its site limit is the site it is on now unless the agent passes `allowed_origins`; any other site asks the person, as a navigation off an opened pane's list does. `eval` and `set_files` are refused exactly as on opened panes (the refusal names the reason and the alternative). The auto-approve grant is not inherited; it is requested for this pane.
7. **Profile rule unchanged:** a taken pane keeps its profile; taking it does not give the agent that profile for anything else, and the agent never silently gets the person's Personal sign-ins.

## Where it goes

- `crates/srv/src/server/browser_owner.rs` (the retake and take rules, the 24-hour record, `former_driver` in the meta), a new small `browser_control.rs` (the person's-presence check, the prompt, `release`), `ui_handlers.rs` (`target_for` stays the one gate; new handlers `retake`, `take_control`, `release`, `handoff_status`), `browser_attention.rs` (handoff ids and status, the take prompt kind), `browser_actions.rs` (new log kinds).
- The host (`crates/cef`): a "last input in this pane" timestamp read by srv; no new CDP work.
- Frontend: `browser-model.ts` and the header (badge states, Give back, the take prompt banner, which reuses the handoff banner).
- `crates/mcp/src/tools/browser.rs` and `tool_schemas.rs`: actions `retake`, `take_control`, `release`, `handoff_status`; `Pane layout` gains `driver`.
- Three PRs: (a) retake, give back, `handoff_status` (no host change); (b) the host's last-input time and the idle rule; (c) take_control, release, the prompt, `driver` in the layout.

## Tests

- Rust unit tests (CI): the retake rule table (gave back / took over during a hand-off / still in use / never lost / another agent's; idle 61 s and 5 s with the idle rule); the profile rule for take_control (own, shared, unshared, Personal native, Personal container, another agent's); `handoff_status` through pending, done, cancelled, abandoned and another agent asking.
- The browser-owner integration tests already in `ui_handlers_tests.rs`, extended: takeover then retake on the same pane id keeps its page URL.
- A live check on a dev build on another host (narko's build ban): take over a pane by hand, press Give back, the agent's next snapshot works; an agent takes a second agent's released pane; the Personal-pane prompt appears and Not now refuses.
- Frontend vitest: the three badge states and the Give back and Allow buttons.

## Risks

- An agent that retakes while the person only paused to read could surprise them: hence the 60-second idle rule, the banner and the always-winning Take over.
- Two agents commandeering each other's panes: the owner must release, or the person must allow.
- The idle check relies on input events reaching the host for the pane; a pane the person only watches counts as idle, which is intended.

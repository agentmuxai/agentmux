# Plan: fix the browser driver's roadblocks (R1–R11)

**Date:** 2026-10-17
**Author:** Naki #2 (naki-naki-2)
**Status:** implemented — PR 1 (dialogs) #5074, PR 2 (action log and Pause) #5076, PR 3 (attention slot) #5079, PR 4 (find) #5075, PR 5 (small fixes) #5077. Turns `REPORT_BROWSER_DRIVER_ROADBLOCKS_2026_10_17.md` into PRs. Not yet run: the §9 acceptance job in a live pane.
**Related:** `SPEC_BROWSER_PANE_NATIVE_DIALOGS_2026_10_17.md` (R1), `REPORT_BROWSER_PANE_BLANK_AFTER_BLOCKED_SIGN_IN_REDIRECT_RCA_2026_10_16.md` (R2), `SPEC_AGENT_DRIVEN_BROWSER_PANES_2026_10_07.md` (§5.5, B3b: action log, Pause, `BrowserDialog`), `SPEC_BROWSER_PANE_BYPASS_2026_10_16.md` (Auto-approve).

## 1. Goal

An agent can run a bulk job of about 75 committing actions on one site in a pane it opened:
- one Auto-approve grant from the person;
- no hangs;
- no blank panes;
- a record afterwards of everything it did.

Today R1 alone stops that job. The rest make it slow (R4), unauditable (R6) or fragile (R2, R9).

**Acceptance test (§9):** that job, run end to end on a test page that reproduces each blocker.

## 2. What the code check changed in the roadblocks log

Each R-item was checked against `main` at 468f89ca5 before planning. Corrections:
- **R3 (fresh panes start signed out): the cause in the log is wrong.** `browser_identity.rs::identity_for_agent` (:145) gives an agent's `open` the person's **Personal** jar when it passes no `profile`. A private jar comes only from `profile: "incognito"`. The likely cause is that the person had never signed in to that site inside AgentMux's Personal jar, or the pane was opened incognito. That needs checking, not building (§7).
- **R7 (the label includes the description): the label isn't the accessible name.** The committing-click label is built by `COMMIT_INFO` JS in `crates/cef/src/browser_api/act.rs` (:180–193) as `aria-label || innerText || value`, cut to 120 characters. `innerText` of a menu item includes its description; `aria-labelledby` and `<label>` are ignored.
- **New, R10:** `snapshot.rs::line_for` (:417) adds a ref to the table before `walk` checks the 40 KB cap (:341). So the line that crosses the cap gets a ref the agent never sees. It is harmless today, but an off-by-one waiting to bite once refs are found by name (§6).
- **New, R11:** `browser_owner::forget` (:162) ends a grant without `publish`. After Take over the pane's `browser:bypass` meta can be left stale for the strip.
- **R2 confirmed at the line:** `browser_allowlist.rs::handle_host_browser_navigation` returns `asked:false` and never replays when *any* request is up (:238–240). `browser_attention::ask` (:214) refuses a second request of any kind.
- **R6 confirmed:** no per-pane action log exists anywhere (srv, host, frontend). Agent actions only reach `tracing` (`ui_handlers.rs` :1465, :598, :648, :936; `browser_bypass_routes.rs` :57–67), and the bypass line has the label but not the URL.
- **R8:** `browser_tool.rs::every_action_is_in_the_schema` (:123) already proves the schema's action enum equals the handler table. So the lag seen was not in the source. It was in what the model loaded (§7).
- **R9 confirmed:** grants end on time, on Stop, on Take over, on pane delete, and on a new owner, but never when the agent that holds one stops or goes quiet.

## 3. The PRs, in order

| # | PR | Fixes | Size | Unblocks |
|---|----|-------|------|----------|
| 1 | Native dialogs (D-1) | R1 | L: host + srv + frontend | the bulk job |
| 2 | Action log and Pause | R6, part of R9 | M | auditing Auto-approve |
| 3 | Attention slot: two levels and a Notice | R2 | M | sign-in with a site limit |
| 4 | Find by name; cheaper snapshots | R4, R10 | M | affordable multi-page jobs |
| 5 | Small fixes | R7, R9, R11, R8, R3 | S each; may split | — |

**Release:** PR 1 is the only one the job needs. After it merges, cut a patch-override release (`scripts/release.sh --as patch`) so the person can upgrade and the job can run. PR 2 should ride in the same release if it is ready within a day: the first unattended run is exactly when a log matters.

**Order inside the work:** PR 1 and PR 3 both add a banner kind (`dialog`, `notice`) and both touch `browser_attention.rs`. PR 1 lands first. Its first commit replaces the per-kind ternaries in `browser-view.tsx` (:252–312) with one `kind → {label, tone, primary, secondary, answer}` table (RCA §7.4), so PR 3 adds a row, not a branch.

**File-size baseline:** `ui_handlers.rs` is at its baseline cap (it failed CI on #5045). New routes go in new modules (`browser_dialog.rs`, `browser_actions.rs`, `browser_find.rs`), wired in one line each.

## 4. PR 1: native dialogs (R1)

Build `SPEC_BROWSER_PANE_NATIVE_DIALOGS_2026_10_17.md` phase **D-1** as written; that spec is the design. The plan adds only the build order and the checks.

1. **Banner kind table** (frontend refactor, no behaviour change); see §3.
2. **Host** `crates/cef/src/client/jsdialog.rs`: `AgentMuxJsdialogHandler` with `on_jsdialog`, `on_before_unload_dialog`, `on_reset_dialog_state` and `on_dialog_closed`.
   - Register it in `client/handlers.rs`'s `wrap_client!` (:29–135), gated `is_browser_pane && !drag_capture`.
   - Keep the parked-callback registry on the pattern of `browser_pane/auth.rs` (`register`/`take`/`cancel_for_block`).
   - Return `false`, so CEF shows its own modal as today, for a pane with no agent owner or one in a hand-off. Nothing changes for people browsing.
3. **Host route** `POST /agentmux/browser/dialog_answer {id, accept, text?}`, checked by `X-Host-Token` like the other host routes.
4. **srv** `server/browser_dialog.rs`:
   - `POST /api/v1/host/browser_dialog` and the policy (spec §5): alert accept; confirm or leave-site accepted only when paired with the agent's own Auto-approve click on the granted origin within 5 s, and never for payment; otherwise ask; prompt cancel; flood cancel.
   - The click pairing reads a "last bypassed click" stamp that `browser_bypass_routes::bypassed_click` writes: pane, origin, time, label.
5. **srv attention:** `Kind::Dialog` in `browser_attention.rs`, answered through the existing host-relayed banner path. A dialog that arrives while another request holds the slot is cancelled and reported. PR 3 later gives it the second level.
6. **Tool results:** the `dialogs` list (spec §6) on `act`, `navigate`, `wait_for` and `snapshot`, so the agent learns what was answered for it.
7. **`owned_panes` push:** the `handoff` list, so the host knows not to intercept during a hand-off.

**Tests:** as in the dialog spec's §11. The ones that matter most:
- the host unit test that an unowned pane returns `false` (CEF's modal, unchanged);
- the srv test that a confirm with no paired click asks, and never accepts;
- the srv test that a paired click on a payment label asks;
- the timeout test: srv is unreachable, so the host cancels after 5 s and the pane is not left hung.

**Manual check** in an isolated `task dev` (never the running instance):
- a local test page with `confirm()`, `alert()`, `prompt()` and `onbeforeunload`;
- run with Auto-approve on and off;
- run with a hand-off up.

## 5. PR 2: action log and Pause (R6; Pause also covers part of R9)

From the 2026-10-07 spec §5.5 and its B3b leftovers.

**srv** `server/browser_actions.rs`:
- A ring of the last 200 entries per pane, in memory: `{ts, agent, kind, url, target, how, detail}`.
  - `kind`: navigate, click, fill, select, check, key, upload, eval, dialog, handoff, auto-approve on/off.
  - `target`: the element's label, or the field name for `fill` (the value is never logged, secret or not).
  - `how`: `allowed`, `approved`, `auto-approved`, `asked-declined` or `refused: <reason>`.
- Recorded from the existing handlers: `handle_ui_browser_act` (:1465), eval (:598), dispatch_key (:648), upload (:936), `bypassed_click` (:57–67), the navigation and popup refusals, and PR 1's dialog decisions.
- `bypassed_click`'s tracing line gains the URL (R6's missing piece).
- Route `GET /api/v1/ui/browser/actions?block_id=` for the pane and for the agent's own `Browser` action `log` (read-only, its own panes only).
- Cleared when the pane is deleted. Kept across ownership changes, so a log survives Take over.

**Frontend:** in `driven-by-strip.tsx`, a "Log" button with a count opens the list in the pane's own flow, like the Auto-approve confirmation and for the same reason: the page is a native window drawn over popovers.

**Pause:**
- A per-pane flag in `browser_owner.rs`; while it is set, `target_block_id` returns 409 "the user paused this pane".
- A Resume button.
- **Pause also pauses an Auto-approve grant's clock** (the grant's `ends_at` moves by the paused time). It is the person's quick, reversible brake, where Take over is final.

**Tests:**
- each handler writes one entry with the right `how`;
- a `fill` entry never contains the value;
- Pause refuses every action, and Resume restores them;
- the log is read-only to agents and limited to their own panes.

## 6. PR 3: two-level attention slot and a Notice (R2)

Build RCA §6 and §7 as written:
- the second level (one Navigation or Dialog question on top of a Handoff or Approval);
- `handle_host_browser_navigation` drops only when a Navigation question is already up;
- `Kind::Notice` (non-blocking), plus the host's local fallback notice when srv can't be reached;
- `info` logging on every branch;
- the typed `Busy { kind }` error from `ask`, replacing the duplicate `waiting_on_user` pre-check;
- one "push the list, then replay" helper.

**Tests:** RCA §6.4. The headline one is a Handoff parked, a Google redirect off the list, the question asked, Allow, and the redirect loads.

## 7. PR 4: find by name, cheaper snapshots (R4, R10)

**`Browser` action `find {pane, role?, name, exact?}`:**
- It runs the same AX tree read as `snapshot_route` (`act.rs` :506).
- It returns only matching lines, each with a ref, plus each match's nearest named ancestor for context.
- It is capped at 20 matches.
- Its refs merge into the pane's `RefTable` like a scoped snapshot's (:589–597), so `click` works on them right away. Nothing else changes in the ref table.

**`scope` on a fresh pane:** a `scope` that names a role and name (`scope: {role, name}`) as well as a ref, so the first read of a page can be narrow.

**R10:** in `walk`, check the cap before `line_for` assigns a ref. Split `line_for` into "format" and "assign ref", and call the second only when the line fits.

**Tests:**
- `find` on a fixture page returns the right ref, and clicking it works;
- no ref in the table is missing from the text (a property test over truncated snapshots).

## 8. PR 5: small fixes (each may be its own PR)

- **R7, the label:**
  - `COMMIT_INFO` takes the name from `aria-label`, then `aria-labelledby`, then the first line of `innerText`.
  - The rest of `innerText` goes to a separate `description` field. The approval banner shows it smaller; payment and verb checks run on the name only.
  - Test: a menu item whose description holds "pay" doesn't count as a payment, and a "Delete" whose description says "safe" still commits.
- **R9, the grant outliving its agent:**
  - In `browser_bypass.rs`, end a grant after 15 minutes with no `Browser` call from the holder on that pane, with reason "no activity". This covers a hung or stopped agent without srv needing to know which.
  - Also end it at once when srv learns the holder's agent process exited. No single hook for that was found on 2026-10-17; the agent-process exit path (`app_api/agent_spawn.rs`, `fleet.rs`) is the place to look. Only add it if one exists, since the idle rule already bounds the risk.
  - The strip says why it ended.
- **R11:** `browser_owner::forget` publishes the cleared grant (or calls `browser_bypass::end_if`), so the strip can't stay stale after Take over. Test: Take over clears `browser:bypass` meta.
- **R8, schema lag:**
  - Find out how the model's loaded schema for `Browser` came to end at `handoff` while the handler accepted `request_auto_approve`.
  - Check whether the client cached `tools/list` across the upgrade's session resume, or whether an older `agentmux-mcp` answered `tools/list`.
  - If it is the cache, send `notifications/tools/list_changed` when the MCP process starts with a schema hash different from the last one it advertised to that session. If it is the process, fix the respawn.
  - Small, but needs a repro first.
- **R3, signed-out panes:**
  - In a `task dev`, sign in to a site in a Personal pane, then let an agent open a pane without `profile`, and confirm it is signed in.
  - If it is, R3 is closed with a tool-description line: "open uses the person's Personal sign-ins; pass `profile` only for a separate identity".
  - If it isn't, it's a bug in `context_for_block` (`browser_pane/identity.rs` :171) and gets its own fix.

## 9. Acceptance: the job that found these

A local fixture site that serves:
- 75 pages, each with a menu, a "Delete …" item whose description text sits inside it, and a `confirm()` behind it;
- one page that redirects off-site during a hand-off;
- one long page over 40 KB.

The fixture lives in the host's test assets. It is not a third-party site.

In a `task dev` build with PRs 1–5:
1. The agent opens a pane limited to the fixture's origin, hands off once, and the person signs in through the off-site redirect. The question is asked (PR 3), not a blank pane.
2. The person grants Auto-approve for 60 minutes.
3. The agent does all 75 deletions with `find` + `click` (PR 4). Every confirm is auto-accepted by pairing (PR 1). No call hangs or times out.
4. The log shows 75 clicks and 75 dialogs, each `auto-approved`, with the URL (PR 2).
5. The agent stops calling the pane, and the grant ends after 15 quiet minutes, with the reason in the strip (PR 5).
6. A payment-labelled button on one page still asks the person, and so does its confirm.

## 10. Coordination

- Claim `crates/cef/src/client`, `crates/srv/src/server/browser_*` and `frontend/app/view/browser` with Presence for each PR, and post a heads-up on the repo channel: the attention and bypass files are shared with anyone working on browser panes.
- The dialog spec, the roadblocks report, the R2 RCA and this plan ship inside the PRs that implement them (PR 1 carries the dialog spec, this plan and the roadblocks report; PR 3 carries the RCA), not as docs-only PRs.

## 11. Not in this plan

- Letting `eval` or Enter/Space through on driven panes (R5): kept refused by design. PR 1 and PR 4 remove the reasons an agent would want them.
- Phases D-2 to D-4 of the dialog spec: after the job runs.
- Named "agent work" profiles beyond what `agents_allowed` already gives (R3), unless §8's check finds a real gap.

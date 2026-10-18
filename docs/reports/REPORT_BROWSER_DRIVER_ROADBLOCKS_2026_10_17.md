# Report: roadblocks hit driving a browser pane as an agent (running log)

**Date opened:** 2026-10-17
**Status:** analysis, a running log. Add to it whenever an agent hits a wall driving a browser pane, so the browser driver can be cleaned up in one pass.
**Author:** Naki #2 (naki-naki-2), at the repo owner's request: "record roadblocks like this to file, so we can come back later to clean up the agentmux browser driver."
**Context of the first entries:** a bulk job of about 75 deletions on one third-party site, driven through a pane the agent opened, with Auto-approve (then called Bypass) on. The job was stopped by R1 before it could run. Build: 0.59.31.
**Related:** `SPEC_AGENT_DRIVEN_BROWSER_PANES_2026_10_07.md`, `SPEC_BROWSER_PANE_BYPASS_2026_10_16.md`, `SPEC_BROWSER_PANE_NATIVE_DIALOGS_2026_10_17.md` (R1), `REPORT_BROWSER_PANE_BLANK_AFTER_BLOCKED_SIGN_IN_REDIRECT_RCA_2026_10_16.md` (R2).

Each entry: what happened, why, what it cost, what would fix it, and where that stands.

---

## R1. A page's native `confirm()` hangs the pane — **blocking**

- **What:** clicking a "Delete …" button on the target site opens the page's own `window.confirm()`. The pane froze; every later `Browser` call timed out after 20 s with an error that doesn't mention a dialog. Auto-approve did nothing for it.
- **Why:** the host has no JS-dialog handler (CEF draws its own modal) and CDP can't see or answer it (`Page.javascriptDialogOpening` doesn't fire; `Page.handleJavaScriptDialog` reports no dialog). `BrowserDialog` was specified on 2026-10-07 and never built.
- **Cost:** the whole bulk job. Each of 74 deletions would need a click by the person on the modal, which defeats Auto-approve.
- **Fix:** own the dialog in the host; decide it in srv by policy. Specified in `SPEC_BROWSER_PANE_NATIVE_DIALOGS_2026_10_17.md`.
- **Status:** specified, not built. Needs a release before the job can run unattended. **Workaround today:** the person clicks OK or Cancel in the pane's modal, once per dialog.

## R2. A hand-off and a site-limit question share one attention slot — blank pane

- **What:** with `allowed_origins` set and the pane handed to the person to sign in, "Continue with Google" cancelled the redirect and nothing asked about it; the pane was left on an empty "Redirecting to…" page.
- **Why:** srv refuses to ask a second question while any request is up on the pane, and returns without logging.
- **Fix and status:** the RCA (`REPORT_BROWSER_PANE_BLANK_AFTER_BLOCKED_SIGN_IN_REDIRECT_RCA_2026_10_16.md`, §6): ask during a hand-off too, never end a stopped navigation without a load, a question or a notice, and consolidate the five places that decide the outcome. Not built.
- **Workaround:** open the pane without `allowed_origins` when it will be used for a sign-in.

## R3. Every fresh pane starts signed out

- **What:** a pane opened for a job on a site the person had signed in to earlier showed the anonymous page. The person had to sign in again through a hand-off.
- **Why (corrected after reading the code):** not a private jar. `identity_for_agent` gives an agent's `open` the person's Personal jar unless it passes `profile`. The likely cause is no prior sign-in to that site in AgentMux's Personal jar, or an incognito open. To be checked; see the fix plan §8.
- **Cost:** one hand-off per pane, per job. Fine for a one-off; costly if a job opens several panes or restarts.
- **Fix ideas:** a named profile for agent work that the person has switched on once ("Agents may use this profile"), reused across panes and restarts; or a per-site "use my signed-in session for this pane" offered in the hand-off. Both are already in the 2026-10-07 spec §6; neither is wired for this flow.
- **Status:** specified; to be checked against the built profile menu.

## R4. Snapshots are large and can't be narrowed before the first one

- **What:** a long conversation page produces a 40 KB snapshot (about 10,000 tokens), truncated. A `scope` argument exists, but refs only exist after a full snapshot, and `scope e1` on a new pane fails with "unknown reference".
- **Cost:** a bulk job over many pages pays for a full snapshot on every page just to find one button. Each page needs two to four.
- **Fix ideas:** `snapshot` with a role/name filter ("the button named 'edited by …'", "the menu item named 'Delete …'") or a CSS-free `find` that returns refs for matches only; or `click` by role and accessible name, without a prior snapshot.
- **Status:** not specified.

## R5. `eval` and Enter/Space are refused on agent panes, even under Auto-approve

- **What:** `eval` on a pane the agent opened is refused ("it could submit a form without the user's approval"); so is Enter or Space. A script to read one value, or to stub `window.confirm`, can't run.
- **Why:** deliberate: they skip the payment and secret checks (muxreview on the Auto-approve PR). It's the right call; the cost is that every workaround that needs script is closed.
- **Cost:** it is why R1 can't be worked around from the agent's side.
- **Fix ideas:** none needed beyond making the cases that need `eval` unnecessary (R1, R4): a read-only `read` action (text of one element by role/name) and a real dialog handler.
- **Status:** by design.

## R6. No record of what the agent clicked

- **What:** during the first run a committing click on a button named "Remove" was logged by srv (`committing click made without asking`), and nothing in the pane or in a tool result says which page or which element it was on. Nothing was found to have changed on the page's timeline, but a reaction or subscription wouldn't show there.
- **Why:** the action log and Pause (2026-10-07 spec §5.5, B3b) are not built. srv's own log has the label and the pane, not the URL or element.
- **Cost:** under Auto-approve, a person can't audit afterwards what was done for them.
- **Fix:** build the action log: time, pane, page URL, element name, how it was allowed (approved / auto-approve / refused), and any dialog answered. Show it in the pane header and keep the last N in srv.
- **Status:** specified, not built. This is the most valuable gap to close next to R1.

## R7. An element's accessible name can include its description

- **What:** the committing-click label for GitHub's revision menu was `Delete revision from history This edit's content will no longer be visible`: the button's visible title plus its description text.
- **Why (corrected after reading the code):** not the snapshot. The click label comes from `COMMIT_INFO` in the host's `act.rs` as `aria-label || innerText || value`, and a menu item's `innerText` includes its description. Label-based checks (payment words, committing verbs) see the whole string.
- **Cost:** none seen yet, but a description that happens to contain a payment word would turn a harmless click into "asks the person"; the reverse is a description that hides a verb. Worth testing with real pages.
- **Fix idea:** classify on the name (the first line / aria-label) and show the description separately.
- **Status:** noted.

## R8. The tool schema the model loads can lag the tool

- **What:** after loading the `Browser` tool, its action list ended at `handoff`; `request_auto_approve` still worked when called.
- **Why:** not investigated: the schema is cached or fetched from an older MCP process than the one handling the call.
- **Cost:** an agent that trusts the listed actions won't know Auto-approve exists.
- **Fix:** check how deferred tool schemas are refreshed after an upgrade; make the handler's schema the single source.
- **Status:** to investigate.

## R9. A stuck helper can keep a pane's grant alive

- **What:** a helper agent that drives the same pane can hang on R1 and leave Auto-approve on for the rest of its time limit.
- **Fix:** the agent ends it (`stop_auto_approve`) when it stops; a hung driver is exactly when it can't. Consider ending a grant when the agent that holds it goes idle for N minutes with the pane still waiting, or showing the person a clear "Auto-approve on, no activity for N min" note.
- **Status:** noted; handled by hand this time.

## R10. A truncated snapshot's last line gets a ref the agent never sees

- **What (found reading the code):** `snapshot.rs::line_for` adds the ref to the table before `walk` checks the 40 KB cap. The line that crosses the cap is dropped, but its ref stays.
- **Cost:** none seen; an off-by-one that matters once refs are found by name.
- **Status:** fix in the plan's PR 4.

## R11. Take over can leave the Auto-approve strip stale

- **What (found reading the code):** `browser_owner::forget` ends the grant without publishing the change, so the `browser:bypass` meta can outlive it.
- **Status:** fix in the plan's PR 5.

---

## Order

Planned in `PLAN_BROWSER_DRIVER_FIXES_2026_10_17.md`:
1. R1 (dialogs);
2. R6 (action log and Pause);
3. R2 (attention slot);
4. R4 and R10 (find by name);
5. R7, R9, R11, R8 and R3 (small fixes).

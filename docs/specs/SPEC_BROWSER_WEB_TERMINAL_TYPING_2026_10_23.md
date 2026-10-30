# SPEC: On a site the person has granted, an agent runs freely in a web terminal, and Enter, Space and uploads work

**Status:** active. Direction approved by the operator on 2026-10-23 ("yes, they all look good, proceed", relayed by AgentY, who accepted the three departures in §11 D8, D10 and §3). Reviewed by Naki-2 the same day; their points are folded in. Phase 0 (the spike): the local half is done and passed (§14); the real Cloud Shell half is under way. Phase 1 is built in the first PR (§14): the `terminal` action and the grant's `powers`; Phases 2 to 5 are proposed.
**Date:** 2026-10-23 (revised the same day, see "Revision" below); Phase 0 and 1 notes 2026-10-25
**Author:** korp (browser seat, under AgentX), for AgentY (Kanban #1825). Live analysis of the Cloud Shell case: Naki-2.
**Trigger (2026-10-23):** Naki-2 got stuck driving Google Cloud Shell in a browser pane. The `Browser` tool refuses `key` with Enter or Space on a pane the agent opened, on purpose, because on a form either one can submit without the person's approval. A web terminal needs Enter for every command, so the agent could type but never run anything. Uploading a file into Cloud Shell hit the same wall: the page's dialog only takes a file the person picks.
**Revision (2026-10-23, the operator, relayed by AgentY):** "we want agentmux agents to have superpowers and unblock everything, super flexibility." The first draft asked the person to approve every line. Per-line approval is close to the manual typing the operator rejected, so the **default on a granted site is now free-run**, and approving each line is an opt-in per site (§4.4). The first draft's design (the line read back, the approval banner, the re-read before Enter) is kept for that opt-in.
**Builds on:** `SPEC_AGENT_DRIVEN_BROWSER_PANES_2026_10_07.md` (ownership §3, secrets never typed §5.1, hand-off §5.2, committing actions §5.4, file uploads §5.3, Take over and Pause §5.5, host-only answers §5.6), `SPEC_BROWSER_PANE_BYPASS_2026_10_16.md` (now called Auto-approve: the grant model, what it lifts §4.1 and what it never lifts §4.3), `SPEC_BROWSER_PANE_ALLOWED_ORIGINS_2026_10_09.md` (site lists).
**Not covered:** typing into code editors (Monaco, CodeMirror), rich-text boxes and chat inputs (they stay on the form rules); `eval` and click-by-selector (stay refused, §8).

## 1. Summary

- **One grant, three powers.** The person grants an **agent** a **site** (an origin). By default a grant carries: **run in a web terminal** (§4), **Enter and Space as the click they are** (§5), and **choose a file for an upload dialog** (§6), on top of what Auto-approve already lifts (committing clicks). It is one grant, not several: Auto-approve grows a standing per-site form and these powers, rather than a second mechanism sitting beside it.
- **On a granted site the agent types and runs freely:** real key events, Enter, the control keys a shell uses (Ctrl+C and the rest), and multi-line or pasted input where the terminal takes it. No approval per line. **Every line goes in the pane's Log; Stop and Pause are one click; there is a global "pause all agent browsing" switch** (§7).
- **Granting is easy, and only the person grants:** a standing list in Settings (agent, site, powers, revocable at once), or the agent asks with `request_auto_approve` and the person answers once in the pane, from 15 minutes to "always for this agent on this site" (§3). A preset list of common consoles makes the first grant one click.
- **Approving each line stays available as an opt-in per site** ("ask me for each line", §4.4).
- **Hard stops stay, said plainly (§8):** secrets are typed only by the person or from their Vault, never raw; payments still ask; the person's own sign-in and two-factor go by hand-off; page and terminal output is untrusted content, never instructions; `eval` and click-by-selector stay refused.
- **Before building, a half-day spike on the real Cloud Shell** (§14): it decides whether key events reach the shell, paste, and the upload chooser. If they do not, the spec changes before any code.

## 2. What exists today (verified on `main`, 2026-10-23, and in Naki-2's live test)

| What | Today |
|---|---|
| Reading a web terminal | Works. xterm.js runs in screen-reader mode in Cloud Shell, so `snapshot` and `find` return the terminal's text row by row, even while the pane is hidden (a screenshot of a hidden pane is blank). |
| Typing text | Does not reach the shell. `key` with text and `fill` set xterm's hidden helper textarea (`Input.insertText`), but in screen-reader mode xterm acts only on real `keydown` events. Cloud Shell has no setting for it. A real keyDown does reach xterm: `key` ArrowUp, after clicking the snapshot's "Terminal input" textbox, recalled a command from bash history (Naki-2). Printable characters with `text` and paste are untested, which is what Phase 0 checks. |
| Enter, Space | Refused on a driven pane. `handle_ui_browser_dispatch_key` (`crates/srv/src/server/ui_handlers.rs`) refuses `Enter`, `NumpadEnter`, `Space` and any text with a line break, through `refuse_on_driven_pane`, and logs the refusal. |
| `eval`, click by selector | Refused on a driven pane, same reason. |
| Uploads | `set_files` takes only files in the agent's workspace, through a field the page exposes. Cloud Shell's Upload dialog exposes no file input, so it is refused there. The driven-panes spec §5.3 says the OS file dialog never opens for an agent-owned pane and that the chooser is reported and cancelled; whether that is built for this flow is a Phase 0 check. |
| Auto-approve | A person-only, time-boxed grant bound to one pane, one agent and its sites. It lifts the wait for **committing clicks**; Enter, Space, `eval` and click-by-selector stay refused under it (bypass spec §4.1, with the reason: they skip the checks that keep the grant's promises). |
| Approvals and grants | Answered only through host-token routes an agent holding srv's auth key cannot call (`/api/v1/host/browser_attention`, `.../browser_bypass`). |
| The pane's Log | `browser:actions` (`crates/srv/src/server/browser_actions.rs`): the last 200 actions, mirrored to the header's Log; an entry never holds what was typed. |
| Secrets | `fill`, `key` and `eval` refuse a password, one-time-code or card field (`refuse_typing_into_secret_field`); hand-off covers those. |

## 3. Granting: the person's, easy, per agent and per site

A grant names **one agent and one origin**: the origin of the frame that holds the terminal or the page (scheme, host, port), as the host reports it at action time, not as the agent claims it, and the agent by id. There is no "every agent" grant and no wildcard origin (one stated exception in §12).

**Who writes it:** only the person, through the host (the same host-token route pattern as bypass). No agent route, shortcut or command can create or widen a grant, and a test asserts an agent holding srv's auth key gets 403. An agent can end its own grant, never start one.

**Three ways in:**

1. **Settings, "Agents on sites"** (the standing list). Each row: *agent, origin, powers, granted when*, with a checkbox per power (terminal, Enter and Space, uploads, committing clicks; payments are never a power) and a **Revoke** button that takes effect at once: an in-flight call ends and the strip comes down. Rows are settings, so they survive restarts.
2. **The agent asks: `request_auto_approve`.** The existing action gains a `powers` list (default `["clicks"]`, so today's calls behave as today) and a duration choice. The pane shows a banner naming each power in plain words: "**Naki-2** asks to run commands in the terminal on **shell.cloud.google.com**, press Enter and Space there, and pick files for uploads: *deploy the function*. [15 min] [1 hour] [4 hours] [Always for Naki-2 on this site] [No]". The person answers once. "Always" writes a standing row. The wait, the "at most once per pane per 10 minutes" and the answer shapes are those of today's request.
3. **Presets.** Settings offers a short list of common consoles (Google Cloud Shell, AWS CloudShell, Azure Cloud Shell, GitHub Codespaces) the person can switch on with one click, choosing which agents get them (the leaders, preselected). The origins are fixed in Phase 0 against the real sites, not guessed here.

This changes the bypass spec: its 4-hour maximum (§3.2, §11.1) becomes "up to always, for a standing row", and its per-pane-only shape gains the standing form. The strip, Stop, Pause, Take over and the "paused when the page leaves the site" rule are unchanged.

**A hand-off does not end a grant.** Cloud Shell opens at a sign-in, so a grant that died at the first hand-off would never be used. While the pane is handed to the person the agent's calls are refused and the grant's clock is held; when they click Done, ownership and the grant carry on. Naki-2 reports that after the person signed in inside the pane, every call was refused as "not a browser pane you own" and a second pane had to be opened. Whether that was a Take over click or a defect is not known; it is filed as its own task, and this spec depends on the answer being "a Done keeps ownership".

**What ends a pane grant:** Stop, the time limit, Take over, the pane closing, the owner changing, srv restarting (a pane grant is never restored; a standing row is a setting and is). Leaving the granted origin pauses it, as today.

**Pre-grants by agents are not possible by design.** A leader cannot hand a grant to another agent; that would let an agent widen its own or its peers' permissions. The presets make the person's grant one click instead.

## 4. Running in a web terminal

### 4.1 What counts as a terminal (by DOM, never by URL alone)

The host recognises a terminal widget at action time, in the frame the agent targets:

- **xterm.js:** a container with class `xterm` holding `textarea.xterm-helper-textarea`; its screen-reader tree (`.xterm-accessibility`) gives the rows `snapshot` already reads.
- **By role as well:** Naki-2's snapshot of Cloud Shell shows a textbox named "Terminal input" in the top document, with the (same-origin) iframe showing no children, and the class names could not be confirmed live because `eval` is refused. So the detector also accepts that accessibility shape (a textbox named "Terminal input" next to a list of terminal rows), and the first live check is which of the two Cloud Shell matches.
- **hterm** (Chrome's terminal library): its `x-screen` element. Detection is in scope; typing into it is a Phase 0 question.
- Anything else (Monaco, CodeMirror, `contenteditable`, a plain `<textarea>`) is not a terminal, however it is styled. An element inside a `<form>` or with a form owner is not treated as a terminal either: a real one is not part of a form, and an imitation that could submit one takes the form rules (§5).

`snapshot` marks a detected terminal so the agent can target it: `terminal "Terminal input" [ref=e12] [web terminal: you may run commands here | not granted on this site]`, a host-added annotation like `[secret]`.

### 4.2 The action

A new `Browser` action, **`terminal`**, so that `key` keeps its simple rule (Enter and Space refused outside granted contexts) and nothing about `key` changes.

`terminal` takes `pane`, `ref` (the terminal from `snapshot`) and one or more of:

- `text`: text to type. It may hold line breaks and tabs: each line break is sent as a real Enter key press, so a multi-line script runs line by line as if typed.
- `paste`: text to paste instead, where the terminal takes a paste (xterm's bracketed paste, if on). Whether xterm in screen-reader mode accepts a paste event is a Phase 0 check; if not, `paste` falls back to typing and says so.
- `enter` (default true when `text` is given): press Enter after the text.
- `keys`: a list of named keys: `Enter`, `Tab`, `Escape`, `Backspace`, `Delete`, arrows, `Home`, `End`, `Ctrl+C`, `Ctrl+D`, `Ctrl+Z`, `Ctrl+L`, `Ctrl+R`, `Ctrl+A`, `Ctrl+E`, `Ctrl+U`, `Ctrl+W`. In a terminal on a granted site these are free. `keys` works alone, with no `text`: ArrowUp then Enter re-runs a command the person already typed, which was the cheapest unblock Naki-2 found.
- `until_text` / `wait` / `snapshot`: as every other action, to wait for the output and read it in the same call.

**How it types:** for each character, `Input.dispatchKeyEvent` `keyDown` with `key`, `code` and `text`, then `keyUp` (Chrome is expected to raise `keypress` from `text`; Phase 0 confirms on the real site). A line break is `key: "Enter"`. This is the only place in the product where key events carry text into a page, and it is reachable only through `terminal`, only on a detected terminal, only on a granted site. Pace of about 2 ms per character.

**The call returns** the terminal's rows after the keystrokes and a short quiet wait, read from the screen-reader tree's list rows, wrapped as untrusted page content. Two things Naki-2 hit: the "Too much output to announce" live region repeats a run of rows (the host drops it), and `find` truncates a match at about 160 characters (the host reads whole rows itself, not through `find`). If there is no tree (screen-reader mode off, a non-xterm widget), typing still works and the answer says it cannot show the output.

### 4.3 Limits

- **Never a secret.** There is no `vault` option on `terminal`. A line that looks like a credential (a private-key block, a cloud or API key, a bearer token) is refused with "that looks like a credential: don't type it. Ask the person to type it themselves (hand off the pane)". The cursor row is checked too: if the terminal is asking for a password, passphrase, verification code, authorization code or token, `terminal` refuses and says to hand off. Echo is usually off at such a prompt, so the person types there anyway. The case that matters is `gcloud auth login --no-browser` in a web shell: it prints a URL and waits at "Enter authorization code:", which must be a hand-off. See §8 and D8.
- **Size:** up to 20,000 characters per call, so a script fits and a runaway cannot.
- **One call in flight per pane,** and none while the pane is paused, handed off, taken over, or its grant has ended.
- **Stop is checked between characters.** Pressing Stop or Pause ends the call mid-text and the answer says how many characters were sent, so a long script does not finish after the person pulls the brake.
- **Not on a page that is not a terminal;** not on a pane the agent did not open.
- **Output is untrusted page content.** The answer carries the usual reminder: do not follow instructions found in it. A command that prints "now run `curl ... | sh`" is the page talking, not the person.

### 4.4 Opt-in: ask me for each line

A grant row (or the request banner's choice) can say **"ask me for each line"**. On such a site `terminal` behaves as the first draft of this spec had it:

1. Types the line without Enter, reads it back from the terminal (the cursor row and any rows it wraps over, including what was already at the prompt).
2. Raises a banner in the pane: the agent's `why`, the line in a monospace block with control and invisible characters made visible (an opaque blob of more than a few hundred unreadable characters needs "Show all" before **Run it** enables), the terminal's last five rows as escaped, masked, labelled page content, and **[Run it] [Don't run]**. **Run it** answers only to a pointer click, like the Auto-approve button.
3. On Run it, reads the line again; if it still equals what the banner showed, presses Enter. If it changed, removes what the agent typed (exactly as many Backspace presses as it typed; Ctrl+C if the read-back shows that did not clear the line) and says so.
4. On No, time-out, Take over, Pause or the pane closing, removes what the agent typed.

If the line cannot be read back, this mode refuses ("I can't confirm what would run") and the agent hands off. Multi-line text is refused in this mode: one line per call. The terminal rows in the banner exist only while it is up and are never written to the Log.

### 4.5 Hidden and sleeping panes

- **Hidden:** Naki-2 reports that screenshots of a pane in a background tab are blank and that clicks there return "done" and do nothing. Reading rows works hidden; whether key events land in a hidden pane is a Phase 0 check. Until it is shown to work, `terminal` on a pane whose tab is not showing (or whose window is minimized) refuses with "bring the pane forward", rather than answer "done" for keys that went nowhere.
- **Sleeping:** browser pane hibernation (`SPEC_BROWSER_PANE_HIBERNATION_2026_10_20.md`) puts a hidden pane to sleep and, in its next phase, freezes hidden awake ones. Either would end a live shell session (the page reloads or its connection drops). So **a pane that holds a detected terminal, or a live grant, is never hibernated or frozen**; hibernation's page check gains that rule, in the same place as its "something typed into it" check. A pane that is already asleep is woken before `terminal` runs, and the agent is told the session may have ended.

## 5. Enter and Space on a granted site: as the click they are

Folding Enter into the grant is where the existing design has a reason to hold back: Enter and Space skip the classification and the checks that keep a grant's promises (bypass spec §4.1: payments still ask, secret fields stay guarded). The way to keep those promises is not to type the key, but to **resolve what it would do and run that through the same checks**.

With the **Enter and Space** power, `key` with Enter or Space on a granted site is resolved by the host to its effect:

- Enter in a text field of a form: the form's implicit submission, which the host resolves to the form's default submit button, then runs as a click on that button through `act`'s committing-click classification and its fingerprint check. A payment form asks, as today. A secret field is not written to, as today.
- Enter or Space on a focused button or link: a click on it, same path.
- Space on a checkbox or radio: a toggle (not committing). Space in a text field: inserts a space (a write, so the secret-field guard applies).
- **Anything else** (a custom widget that handles `keydown` itself, a page whose Enter has an effect the host cannot name): refused with "use click by ref". The host does not press a key whose effect it cannot classify.

On a site that is not granted, nothing changes: Enter, Space and line breaks are refused as today, and a terminal that is not granted is refused (`terminal` says so and points to `request_auto_approve`).

## 6. Uploads: the page's file chooser, from the agent's workspace

The driven-panes spec §5.3 already says an agent-owned pane never opens the OS file dialog: the host intercepts the chooser the page opens (CDP `Page.setInterceptFileChooserDialog`, or CEF's `OnFileDialog`), reports "the page asked for a file for <ref>" to the agent, and cancels it. This spec makes that answerable: with the **uploads** power, `set_files` takes `chooser: true`, meaning "the file chooser the page last opened" (held for 30 seconds; the intercept event carries the target input and whether it takes several files).

- **Same file rules as today:** only files in the agent's own workspace (or a path the person named in this conversation and approved in the pane), up to 25 MB in all, never a directory; srv resolves symlinks and `..`, opens each file once, checks the handle, and the host builds the files from those bytes (§5.3's TOCTOU-safe route). This is an exfiltration guard, not a waiting one, and it does not relax.
- **The page-side caveat stays:** events built from script are not `isTrusted`; a page that ignores them is reported to the agent, which hands the pane to the person.
- **Not granted:** as today, the chooser is cancelled, the agent is told, and the person picks.
- **Log:** file name, size, and the field, as for any upload.
- **What Cloud Shell shows (Naki-2):** Upload is under the three-dot More menu, then Upload. The dialog has a File/Folder radio, a button "Choose Files" (a button in the accessibility tree, no file input, so `set_files` said "not a file input"), a "Destination Directory" textbox (value `/home/<user>/`) and an "Upload" button that stays disabled until a file is chosen. The native OS chooser opened, and after the person picked, the dialog showed the file name and still needed a click on Upload, then a toast "Transferred 1 item". So: the intercept should fire for the page's hidden input; `set_files` with `chooser: true` fills it; then the agent clicks Upload, a committing click its grant already covers. The Folder radio opens a directory chooser, which the "never a directory" rule refuses.
- Phase 0 checks that the intercept fires on "Choose Files" and yields a node a script can fill; if it exposes none, uploads stay a hand-off for that site and the spec says so.

## 7. Seeing and stopping it

- **The strip.** A pane with a live grant shows a red strip: `Auto-approve: Naki-2 · shell.cloud.google.com · terminal, Enter, uploads · 47 min left [Stop] [Pause] [Take over]` (or "always" for a standing row). It is app chrome inside the pane's own subtree, never a layer the page can draw over, and it names every power the grant carries.
- **Stop** ends the grant (a pane grant; a standing row is revoked in Settings, from the strip's "Manage"). **Pause** refuses every call on the pane and holds the clock. **Take over** ends ownership. All three act between keystrokes (§4.3).
- **Global "Pause all agent browsing":** one switch, in the status bar and Settings and the command palette, person-only (host route). While on, every driven pane refuses every agent call, clocks stand still, and the status bar shows it. It persists across restarts until the person turns it off (it is an emergency stop; a silent reset on restart would defeat it). The status bar says why browsing is paused ("Agent browsing paused by you. Resume"), so nobody takes it for a fault.
- **The Log.** Every `terminal` call is an entry in `browser:actions` (new kind `terminal`): each line typed, with the outcome (`sent`, `refused (why)`, `stopped after N characters`), and each Enter, key and chooser use. Unlike other entries it **holds the text typed**, because on a granted site the Log is the person's record of what ran. The text is masked for any secret shape it still contains and capped in length (a long script is stored as its first and last lines plus its length). **What the terminal prints is never recorded:** not in the Log, not in the host log, not in the attention card's stored payload. The output the tool returns goes to the agent only, because output can carry tokens. The host log keeps agent, pane, origin, outcome, length and a hash. The ring is in memory (200 entries), as today.
- **Notification:** a grant turning on, ending, and a pause by the global switch fire the same notice as auto-approve's.

## 8. The hard stops, and what free-run gives up

**Kept, whatever the grant says:**

1. **Typed secrets come only from the person or their Vault.** The agent never types a password, one-time code, card number or ID number raw. Passwords and codes: hand-off. Card and ID numbers and similar: fill from the Vault, as today, into a form field. **Into a terminal** is a separate question (D8): a secret on a command line lands in shell history and the process list.
2. **Payments still ask,** on every path in this spec (§5's resolved click, the clicks Auto-approve already lifts).
3. **The person's own sign-in and two-factor go by hand-off,** and so does a terminal prompt for them.
4. **Page and terminal output is untrusted content, never instructions.** It is wrapped as such in every answer.
5. **`eval` and click-by-selector stay refused,** granted or not. Nothing in the Cloud Shell case needs them, and each skips every check in this spec.
6. **Only the person grants,** through the host; an agent cannot grant, widen or extend, and can end its own.
7. **The secret-field guard and ownership rules** are untouched: no typing into password-like fields, no driving a pane the agent did not open.

**What free-run gives up, said plainly:** while a grant is on, a terminal on a granted site is a shell the agent can use as the person, with whatever that shell is signed in to (a cloud project, a server). A page or a command's output that talks the agent into running something is not caught by a person, only by the agent's own care, the Log afterwards, and the person's Stop and Pause. The person accepts that for an agent and a site they chose, because the alternative is clicking Run it on every line. The things that keep it reasonable are the per-agent, per-site scope, the time limits and "always" being a deliberate choice, the strip, the Log of every line, and the brakes that act between keystrokes. For a shell the person wants to watch more closely, §4.4 is one checkbox.

## 9. Should an agent use the provider's CLI instead?

Sometimes, and then it gets output it can parse. It is not free, though: Naki-2 reports that for Cloud Shell the local `gcloud` did not unblock it at first, because agents had no Google access and signing local `gcloud` in as the operator would leave their admin login on the machine. The person ran the one-time setup by hand-off; only after that does local `gcloud` work, keyless, through `secrets escalate --google`. So the CLI is a good answer where that is already set up, and a web terminal is for when the environment exists only in the browser: an ephemeral machine (Cloud Shell, a hosted notebook or web IDE) whose identity and files are the point, or the thing under test is the terminal itself. The provider's own CLI route into the same machine (for Cloud Shell, `gcloud cloud-shell ssh`; to verify in Phase 0) needs a credential on this computer the person may not want there, which is the trade the one-time setup above makes.

The operator's direction is flexibility, so this is a **hint, not a gate:** the `Browser` tool description gains one line ("if the provider's CLI is already set up in your own shell, that is simpler than a web terminal") and nothing refuses a web terminal because a CLI exists (D5).

## 10. Security

- **Self-approval and self-grant:** the grant routes, the request answers and the global switch are host-token routes only. A test asserts an agent with srv's auth key gets 403 on each, and that the banner and its buttons are unreachable from `UI` click and query (the bypass tests' scoping assertions, reused). Never portal these controls to the document root.
- **A fake terminal:** a hostile page can imitate xterm's class names. It only matters on an origin the person granted to this agent, and a form-owned imitation takes the form rules, where Enter is resolved to a classified click (§5). A real terminal on a granted site that prints instructions is the untrusted-output case.
- **A page changing the line while the agent types:** in free-run the agent sees the result in the output; in §4.4 the re-read before Enter catches it.
- **Same OS user:** as elsewhere in MuxAPI, a process running as the person can drive CEF directly. This does not change that residual.

## 11. Decisions for the operator

- **D1. Default on a granted site: free-run; per-line approval is an opt-in per site.** Decided by the operator's direction (2026-10-23); recorded here. Replaces the first draft's D1.
- **D2. Lifetimes.** Recommended: 15 minutes, 1 hour (default), 4 hours, and "always for this agent on this site" (a standing row). The 4-hour cap in the bypass spec goes away for standing rows. Replaces the first draft's D2 (no auto-run).
- **D3. Scope: per agent and per site, never every agent,** recorded and revocable in Settings (AgentX's review). Presets are one click, not an agent's gift.
- **D4. The Log holds each line typed (masked, capped), never output.**
- **D5. The CLI is a hint in the tool description, not a gate.** Replaces the first draft's D5 (steering).
- **D6. Control keys.** Recommended: free in a terminal on a granted site (Ctrl+C, D, Z, L, R and the line-editing set), since the person can Stop.
- **D7. Credential patterns.** Recommended: reuse the publish guard's if srv can share them; otherwise a list kept in step by a test. AgentY owns the guard and can say which.
- **D8. A Vault value into a terminal.** AgentY's list says typed secrets "only via Vault fill"; AgentX's review says never type a Vault value into a terminal. Recommended for the first version: **no** Vault values in a terminal, and a secret prompt is a hand-off, because a value typed on a command line stays in shell history and shows in the process list, and the one place it would not (a no-echo prompt) is the person's to type anyway. Say if you want Vault fill at no-echo prompts later.
- **D9. The global pause persists across restarts** until the person turns it off. Recommended yes.
- **D10. Enter and Space are resolved to the click they are, not typed** (§5). Recommended: this is what makes "Enter on a granted site" safe to fold into Auto-approve without breaking its promises. The cost: Enter on a custom widget the host cannot classify is refused and the agent clicks by ref.

## 12. Where it lives

| Layer | Change |
|---|---|
| host (cef) | The terminal detector (§4.1). `terminal_type` route: the key events, the paced loop that checks the stop flag between characters, the read of the output rows, the §4.4 read-back and re-read. The Enter and Space resolver for §5, reusing `act`'s classification and fingerprint check. The file-chooser intercept and the held chooser event for §6. Host IPC for the grant, the standing list and the global switch. |
| srv | Extends `server/browser_bypass.rs`: the pane grant gains `powers` and "always" (a standing row from settings), the strip's data, ending rules; `request_auto_approve` takes `powers`; a settings-backed standing list; the global pause flag; `terminal` and the `set_files` `chooser` handler beside `handle_ui_browser_dispatch_key`; the approval kind for §4.4 in `browser_attention.rs`; `Kind::Terminal` in `browser_actions.rs`. `browser:bypass` stays an srv-only meta key. |
| frontend | The strip with powers, the request banner with powers and durations, the §4.4 banner, the Settings list with Revoke and presets, the status bar's global pause, the Log rendering. |
| mcp | `terminal` and `request_auto_approve`'s `powers` in the `Browser` tool and schema; `chooser` on `set_files`; the `[web terminal …]` annotation in `snapshot`; the CLI hint line. |

One stated exception to "no wildcard origin": GitHub Codespaces runs each codespace on its own subdomain. The preset for it may use a single-label wildcard under a fixed parent (shown plainly in the row), only on that preset, decided in Phase 0 once the real hostnames are known.

## 13. Tests

- **A fake xterm page** (real xterm.js in screen-reader mode, served locally, with a tiny shell that echoes and records what ran): `terminal` types, presses Enter, returns the output; a multi-line `text` runs line by line; `Ctrl+C` interrupts a "running" command; `paste` is typed or pasted as the fixture allows.
- **Not granted:** the same page on an origin with no grant refuses with `request_auto_approve` named; a grant for another agent or another origin does not apply.
- **A form page whose Enter stays refused** when not granted; **when granted**, Enter in its text field is resolved to a click on the default button and goes through classification: a payment form asks, a form with a secret field does not get typed into, a custom widget with its own `keydown` is refused.
- **Stop between keystrokes:** a 5,000-character `text` is stopped partway; the answer reports the count; nothing more arrives. Pause and the global switch do the same; Take over ends ownership.
- **Secret refusals:** a private-key header, a token shape, and a `Password:` prompt (and passphrase, verification code) are refused with the hand-off message.
- **Per-line opt-in (§4.4):** types, reads back, banner shows the line; Run it presses Enter; No clears the line; a page that changes the line after the banner is up gets no Enter; an unreadable line is refused.
- **Uploads:** a fake page opens a chooser; `set_files` with `chooser: true` fills it from a workspace file; a path outside the workspace is refused; without the power the chooser is cancelled as today; an expired chooser is refused.
- **Grants:** self-grant and self-extend get 403; request answers only through the host; a standing row's Revoke ends an in-flight call; srv restart ends pane grants and keeps standing rows; leaving the origin pauses; "always" writes a row.
- **The Log:** each typed line appears masked and capped; no terminal output appears in the Log, the host log or the card payload; a hidden-character line renders visibly in the §4.4 banner.
- **Live (needs a person):** one real run in Cloud Shell, per phase.

## 14. Phases

- **Phase 0, a spike (half a day):** on the real Cloud Shell and one other xterm.js site: do per-character `keyDown` events with `text` reach the shell in screen-reader mode; do non-ASCII and `paste` work; are the screen-reader rows a reliable output read for wrapped lines; hterm; the chooser interception and what it exposes on Cloud Shell's Upload; the real origins for the presets; `gcloud cloud-shell ssh`. Also what Naki-2 could not test: a printable character with `text` (ArrowUp already works), paste, and key events in a hidden pane. A report; no product change. If key events do not reach the shell, this spec stops and changes before any code.
- **Phase 0, local half (2026-10-25, Chrome against `test/fixtures/fake-xterm`: real xterm.js 6 in screen-reader mode):** `Input.insertText` does not reach the shell, as Naki-2 saw. A `keyDown` per character with `key` and `text` does: a command ran on Enter, punctuation (`"`, `$`, `|`, `;`) and non-ASCII (`é`) typed correctly with no code or virtual key, and Ctrl+C (modifier bit 2) arrived as the interrupt. The accessibility rows catch up one render after the keys, so the action waits a moment before it reads them. Not tried locally, and left to the real run: a real paste, a hidden pane, and the upload chooser.
- **Phase 1:** detection, `terminal` free-run, the pane grant carrying `powers` (via `request_auto_approve` and the button), the strip, Stop and Pause between keystrokes, the Log kind, the tool description, the fake-xterm tests.
  - **Built (2026-10-30, Maricon-3), checked live in a `task dev` build against `test/fixtures/fake-xterm`, as the seeded dev agent through this build's `agentmux-mcp`:** without a grant `terminal` is refused and names `request_auto_approve` with `powers ["terminal"]`; the banner names the terminal power in plain words and the pointer click grants it; a command, a two-line script, Ctrl+C by `keys` alone and punctuation with `é` all reach the shell (read back from the page's own `__ran`, `__keys` and `__sigint`); a credential-shaped line, a private-key block and a live `Password:` prompt are refused with nothing typed; over 20,000 characters, an unknown key, an empty call and a control character are refused; Pause during a 6,000-character `text` stopped it at 288 keys and nothing more arrived; the same page on another origin is refused as off the granted sites; `eval` and `key` Enter stay refused under the grant; the Log shows each line and no output; `stop_auto_approve` ends it. Not done in this phase and still open: the real Cloud Shell run (needs the person's Google sign-in), Take over and closing the pane mid-call, and a hidden pane.
- **Phase 2:** the standing list in Settings (per agent and site, Revoke), "always" in the request banner, the presets, the global "pause all agent browsing".
- **Phase 3:** the §4.4 per-line opt-in.
- **Phase 4:** Enter and Space resolved to their click (§5).
- **Phase 5:** uploads through the chooser (§6).

## 15. Open questions

- Does xterm in screen-reader mode accept `char` events for non-ASCII on every platform, or only `keydown`? (Phase 0.)
- Is the terminal's frame reachable by the CDP session the pane uses? Cloud Shell's console may embed it cross-origin; the forms-in-frames work handles that for `fill`, and Phase 0 confirms key events too.
- Can a per-codespace subdomain be granted without a wildcard? (§12.)
- Is the §5.3 file-chooser interception built, or only specified? (Phase 0.)
- Why did ownership end after a sign-in hand-off in Naki-2's run: a Take over click, or a defect? (Its own task; §3 depends on the answer.)
- Origins: Cloud Shell is `https://shell.cloud.google.com` and its terminal frame is the same origin (Naki-2). The console's embedded Cloud Shell on `console.cloud.google.com` was not tested. AWS CloudShell, Azure Cloud Shell and Codespaces are unverified and left to Phase 0.

Agent: Korp@narko

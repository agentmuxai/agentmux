# SPEC: agents fill forms inside iframes, with a visual fallback for what the snapshot can't see

**Status:** active. F1 (SSN/ITIN/TIN fields are secret) shipped in #5047. F2 built in the frames PR: the frame-aware CDP transport, the joined snapshot with `f1e12` references, and click, fill, select and check inside same-process and out-of-process frames, verified live on a fixture page (same-origin, cross-origin). Not built yet: frame origin rules (§5.3), uploads, scope and `gone` waits inside frames, frame waits, the visual fallback (§4.6), the fill ladder and comboboxes (§4.4, §4.5). One detail differs from §4.1: a frame in another process is found through that process's own frame tree (its root frame names its parent), because the page's frame tree doesn't list it.
**Date:** 2026-10-16
**Author:** Korp@narko, at the operator's request
**Trigger (operator, 2026-10-16):** "we may hit a future situation like this, forms where the site has a esoteric setup. we want to update agents in agentmux with the tooling to handle form filling in these situations too. research best pratices online, write a spec to file."
**Motivating case:** a multi-step online form whose whole wizard sits inside an iframe. The agent's snapshot showed a bare `- Iframe` and nothing to act on, so the user typed every field while the agent read screenshots.
**Builds on:** `SPEC_AGENT_DRIVEN_BROWSER_PANES_2026_10_07.md` (snapshot, references, act by reference, secret guard, approval, hand-off; §4.1 names frame references as a follow-up), `SPEC_BROWSER_PANE_ALLOWED_ORIGINS_2026_10_09.md`, `SPEC_CEF_CDP_IN_PROCESS_AND_DEBUG_PORT_OPT_IN_2026_09_25.md` (the in-process CDP channel), `SPEC_MUXAPI_TOOL_CONSOLIDATION_2026_10_15.md` (the single `Browser` tool with actions).

---

## 1. Summary

1. **The snapshot and every action reach the top frame only** (§2). A form inside any iframe, same-origin or not, is invisible to the agent and can't be acted on. Government, bank, payment, insurance and enterprise sites commonly put forms in iframes.
2. **Make frames first-class** (§4.1 to §4.3): the CDP channel learns sessions and events, the snapshot joins every frame's accessibility tree under its `<iframe>`, and references inside a frame carry its prefix (`f1e12`), as Playwright MCP's do. Click, fill, select and check work on those references exactly as on top-frame ones.
3. **Fill the fields that need more than inserted text** (§4.4, §4.5): masked inputs that read keystrokes, custom comboboxes whose options render elsewhere in the page, native date inputs.
4. **A visual fallback for what has no accessibility tree** (§4.6): a `screenshot` action on the `Browser` tool, and `locate` / `click_at` actions that turn a point on the screenshot back into a reference, so every safety rule still applies. No raw coordinate typing.
5. **Every safety rule holds inside frames** (§5): secret fields are found and refused in any frame; **SSN and taxpayer-ID fields become secret fields**; committing clicks inside a frame need the user's approval, and the prompt names the frame's origin; a frame from an origin outside the pane's allowed list is shown but not actable; screenshots given to the agent black out secret fields.
6. **Acceptance:** a local fixture page with the same shape (§7).

## 2. What exists (verified on `main` at `52e4e3c7e`, 2026-10-16)

| Piece | State |
|---|---|
| Snapshot | `Accessibility.getFullAXTree` with no `frameId` (`crates/cef/src/browser_api/act.rs:476-477`): the root frame's tree only. CDP: "If omitted, the root frame is used." |
| Rendering an iframe | `Iframe` is in neither `INTERACTIVE_ROLES` nor `TRANSPARENT_ROLES` (`snapshot.rs:21-32`), so `line_for` prints a bare `- Iframe` (`snapshot.rs:276`); `walk` skips child ids it can't find (`:254-259`). |
| References | `e{n}` → `backendDOMNodeId`, one table per pane, checked against the top URL (`act.rs:30-65, 513-524`). `backendNodeId`s are not unique across frames. |
| Resolving a reference | an isolated world created on the **main frame** (`isolated_context`, `act.rs:313-341`), then `DOM.resolveNode`. |
| Click | `scrollIntoView` + `getBoundingClientRect` in the isolated world, then `Input.dispatchMouseEvent` at main-frame CSS pixels (`act.rs:115-120, 383-394`). The browser hit-tests these, so a point over an out-of-process iframe does reach it. |
| Fill | focus, select all, `Input.insertText`; if the read-back differs, the native prototype setter plus `input`/`change` (`act.rs:614-642`). |
| CDP transport | one browser-level DevTools session via CEF `SendDevToolsMessage`; replies matched by `id`, events ignored (`crates/cef/src/browser_api/cdp.rs:88-117, 157-224`). No `sessionId`, no `Target.setAutoAttach`, no frame enumeration anywhere in `crates/cef/src`. |
| Secret guard | `is_secret` (`snapshot.rs:77-105`): `type=password`, `autocomplete` of passwords, one-time codes and `cc-*`, and names like password, otp, cvc, card number. **No SSN or taxpayer-ID detection.** Typing via `key` refuses when the focused frame can't be inspected (`act.rs:414-418`). |
| Approval of committing clicks | `COMMIT_INFO` in the main frame's isolated world (`act.rs:177-263`), banner and answer through the host (`ui_handlers.rs:1509-1560`). |
| Eval | refused with 403 on driven panes (`ui_handlers.rs:979-1001`). Stays that way. |
| Allowed origins | main-frame navigations and popups only (`crates/cef/src/client/allowed_origins.rs`, `lifecycle.rs:1165-1168`); frames inside the page are explicitly not checked (allowed-origins spec §3). |
| Seeing a frame | only `UI` `screenshot` of the pane (`Page.captureScreenshot{fromSurface:true}`), which includes iframe content. No coordinate click, no `Browser` screenshot. |

## 3. Goals and non-goals

**Goals**

- An agent can read and fill a form in any iframe, at any depth, same-origin or cross-origin, with the same tools and the same guarantees as in the top frame.
- When a page has no usable accessibility tree in some region, the agent can see it and point at it, and its pointing still goes through references and the safety gates.
- Masked fields, custom comboboxes and native date inputs fill correctly, and a fill that didn't take is reported, not silently wrong.

**Non-goals**

- Filling forms rendered entirely on a canvas with no semantics (Flutter web without its semantics tree, games): §4.6 lets the agent see them; acting there is handed to the user.
- Filling PDF forms in Chrome's built-in viewer: it can't be reached from the page. §4.7 tells the agent to hand off or download and fill the PDF offline.
- Raw coordinate typing or `eval` on driven panes. Both bypass the secret guard and the approval gate.
- Camera, microphone and identity-verification selfies: `SPEC_BROWSER_PANE_CAMERA_ACCESS_2026_09_01.md`.

## 4. Design

### 4.1 A frame-aware CDP transport

In `crates/cef/src/browser_api/cdp.rs`:

1. Outgoing messages take an optional top-level `sessionId`. Replies are matched by `(sessionId, id)`.
2. `on_dev_tools_message` also dispatches **events** to subscribers: `Target.attachedToTarget`, `Target.detachedFromTarget`, `Page.frameAttached`, `Page.frameNavigated`, `Page.frameDetached`. (CEF's `OnDevToolsEvent` has no `sessionId` parameter, so events are parsed in `OnDevToolsMessage`, where the observer already hooks in.)
3. For each driven pane, the host sends `Target.setAutoAttach{autoAttach:true, waitForDebuggerOnStart:false, flatten:true, filter:[{type:"iframe"}]}` on the root session, and again on every child session it attaches to: auto-attach is not recursive, so a frame inside a cross-origin frame is only found this way. The filter keeps workers out.
4. The host keeps a per-pane **frame map**: frame id → `{sessionId, parentFrameId, ordinal, url, origin, state}`. `state` is `loading`, `ready`, `detached`, or `blocked` (§5.3). Same-process child frames share their parent's session; out-of-process ones have their own.
5. Domains are enabled per session only as needed (`Page`, `DOM`, `Accessibility`, `Runtime`), and only while the pane is driven. This keeps the "stateless by default" intent of the in-process CDP spec for panes no agent drives.

**Phase F0 checks before building:** that CEF accepts a top-level `sessionId` through `SendDevToolsMessage` (Vercel's agent-browser and browser-use do this over a socket; our in-process channel is untested), and whether the IRS iframe is same-origin or out-of-process.

### 4.2 The snapshot joins every frame

1. Walk the frame map from the root. For each frame, call `Accessibility.getFullAXTree{frameId}` on the frame's session.
2. Graft each child tree under its owner `Iframe` node (`DOM.getFrameOwner`, or the AX node's `frameId`), indented like any child.
3. Each `Iframe` line names the frame:

   ```
   - main
     - heading "Draft form" [level=1]
     - iframe "Application form" [frame=f1 origin=https://forms.example.gov same-origin]
       - heading "Step 3 of 8: Applicant" [level=2]
       - textbox "Taxpayer's first name" [ref=f1e4] [required] value="…"
       - textbox "Taxpayer's TIN" [ref=f1e7] [required] [secret: ask the user to fill this]
       - radio "United States address (including U.S. Territories)" [ref=f1e8] [checked=false]
       - button "Next" [ref=f1e11]
   ```

4. **References:** the top frame keeps `e{n}`; frame `N` uses `f{N}e{n}`. Internally a reference is `{frame id, session id, backendNodeId, frame URL}`. Frame ordinals are assigned in document order and stay stable for the life of a frame's document, so `f1` stays `f1` across snapshots while the form moves through steps. A reference fails ("take a new snapshot") when its own frame navigated, not only when the top URL changed.
5. **Never a silent bare line.** A frame that can't be shown says why: `[frame=f3 loading]`, `[frame=f3 not shown: over the frame limit]`, `[frame=f3 not actable: origin https://ads.example is not on this pane's allowed list]`, `[frame=f3 not shown: PDF viewer, see §4.7]`.
6. **Limits:** at most 16 frames and depth 4 per snapshot by default, inside the existing 40 KB cap. Frames with no interactive content and no text (tracking pixels, zero-size frames) are listed in one summary line, not expanded. `scope` works on a frame reference too (`scope: "f1"` shows only that frame).
7. **Wait for frames:** `until_*` conditions on actions wait for a frame's document to be ready when the snapshot follows, so a wizard step loading inside an iframe doesn't produce an empty frame. This covers Stagehand's known race of acting in a cross-origin frame whose main world isn't ready yet.
8. Shadow DOM needs nothing new: the accessibility tree is computed on the flattened tree, so open and closed shadow content already appear and resolve by `backendNodeId`. The fixture (§7) checks it.

### 4.3 Acting inside a frame

- **Resolving:** the isolated world (`agentmux-agent`) is created **in the reference's frame**, on its session, and cached per frame document. `DOM.resolveNode` and `Runtime.callFunctionOn` run there.
- **Click:** `scrollIntoView` and `getBoundingClientRect` in the element's frame, then add the offsets of each owner `<iframe>`'s content box up the chain to get main-frame coordinates, and send `Input.dispatchMouseEvent` on the root session as today (the browser routes it to the right process). Before sending, `DOM.getNodeForLocation` at that point must return the element or one of its descendants. If it doesn't (an overlay, a cookie banner, a transformed frame), the click is refused with what is on top instead.
- **Typing (`fill`, `key`):** `Input.insertText` and `Input.dispatchKeyEvent` are **sent on the session that owns the focused frame**. Sent on the root session while focus is inside an out-of-process frame, they are silently dropped; this is the trap that broke other agent browsers.
- **Read-back** runs in the element's frame, as today.
- `select` and `check` run in the element's frame's isolated world, as today.

### 4.4 Filling fields that need more than inserted text

`fill` tries, in order, and reports which one worked:

1. Native `type=date`, `time`, `datetime-local`, `month`, `range`, `color`: set through the element's own prototype `value` setter, then `input` and `change` (the formats are fixed: `YYYY-MM-DD` and so on; the tool converts common forms like `October 15, 2026` and says so).
2. Default: focus, select all, `Input.insertText` (today's path).
3. If the field has a mask (an `inputmode`, a `pattern`, a known mask library's attributes) or step 2's read-back differs: clear it and type the text **key by key** with `Input.dispatchKeyEvent`, because mask libraries read `keydown`.
4. Last resort, as today: the native setter plus `input` and `change`.

The result always carries the value read back. If it still differs (a mask reformatted `12345-0000`, or dropped characters), the result says so plainly instead of reporting success.

### 4.5 Custom comboboxes and listboxes

- `select` on a reference that is a `combobox` but not a native `<select>`: click it (or press Alt+Down), wait for `aria-expanded=true`, find the option **anywhere in the frame's document**, not only under the control (Angular CDK, Ant Design and others render options in a portal at the end of `<body>`), click it, and read back the combobox's value.
- If no option matches: type the text into the combobox and choose the first matching option with Down and Enter, the APG keyboard pattern; then read back.
- The snapshot shows `[expanded]`, `[haspopup=listbox]` and, for an open listbox, its options with references.

### 4.6 The visual fallback

For regions with no usable tree: canvas UIs, images of buttons, controls whose accessible names are empty or wrong.

| Action | Does |
|---|---|
| `Browser` `screenshot` (`pane`, optional `ref` or `frame` to clip) | captures the pane, as `UI` `screenshot` does, and returns the image path plus its scale (screenshot pixels per CSS pixel). Secret fields are blacked out (§5.1). |
| `Browser` `locate` (`pane`, `x`, `y` in screenshot pixels) | `DOM.getNodeForLocation` at that point, repeated on the child session for out-of-process frames, and returns the reference of the nearest interactive element (`f1e7`) and its snapshot line. Nothing is clicked. |
| `Browser` `click_at` (`pane`, `x`, `y`) | `locate`, then a click **on the located reference**, through every rule a reference click has: committing clicks need approval, the hit-test check of §4.3, frame origin rules. If the point has no element (a pure canvas), the click is sent at the point only after the user approves it in the pane, like a committing click, and the result says the target was unknown. |

There is no `type_at`. To type, the agent locates a field and fills its reference, so the secret guard always sees the field. A canvas with no field is handed to the user.

Optionally (phase F4), `screenshot` with `marks: true` draws each visible reference's label on the image (set-of-marks), so the agent can name a reference directly from what it sees.

### 4.7 PDF viewer and other unreachable surfaces

When a frame or the page is Chrome's PDF viewer (or another plugin surface), the snapshot says so (`[not shown: PDF viewer]`) and the tool result suggests the alternatives: download the PDF (`SPEC_BROWSER_PANE_DOWNLOADS_2026_10_14.md`) and fill it offline, or hand off to the user.

## 5. Safety

The driven-pane rules (`SPEC_AGENT_DRIVEN_BROWSER_PANES_2026_10_07.md` §5) apply unchanged in every frame. What frames add:

### 5.1 Secret fields

1. `is_secret` runs on the element's own frame's facts (`DOM.describeNode` on that frame's session), so a password or card field inside a payment iframe is found, not missed.
2. **SSN and taxpayer IDs become secret fields.** A field is secret if its accessible name, `name`, `id`, `aria-label`, `placeholder` or `aria-describedby` text matches, as whole words: `SSN`, `social security`, `TIN`, `ITIN`, `taxpayer identification number`, `taxpayer ID`, or `name`/`id` tokens such as `ssn`, `socialsecurity`, `taxpayertin`, `taxpayerid`. `TIN` matches only as a whole word or a word suffix in camelCase/snake_case (`taxpayerTIN`, `payer_tin`), never inside words such as `setting` or `routing`. False positives are cheap: the user fills the field.
   - A company's EIN is not secret (it is printed on public filings); a field labelled only `EIN` stays an ordinary field.
   - Open question: bank account and routing numbers (§8).
3. While a frame can't be inspected (not attached yet, or `blocked`), typing into it stays refused, as today.
4. **Screenshots black out secret fields.** Before returning an image to the agent, the host paints over the bounding box of every secret field in every frame. A field the user unmasked with a show/hide button stays hidden from the agent.
5. During a hand-off, `screenshot`, `locate` and `click_at` return 409, like every other action, so the agent can't watch the user type.

### 5.2 Committing clicks inside frames

- `COMMIT_INFO` runs in the clicked element's frame; the form summary and fingerprint are that frame's form.
- The approval banner names the frame when it isn't the top frame: "Korp wants to click **Submit** in a form from https://forms.example.gov inside https://www.example.gov."
- The re-run check (the form didn't change since approval) compares the same frame's fingerprint.

### 5.3 Frame origins

- A pane's allowed origins (`SPEC_BROWSER_PANE_ALLOWED_ORIGINS_2026_10_09.md`) still don't stop frames from **loading**: that spec is not a network sandbox, and this one doesn't make it one.
- They do decide what the agent can **act on**. A frame whose origin isn't on the list is `blocked`. Its contents are shown so the agent can tell the user what's there, but its lines carry no references, and the frame's line says why. The user can allow the origin from the banner, as for a navigation, after which the frame becomes actable.
- A pane opened without a list keeps today's behaviour: every frame is actable.
- A cross-origin frame is labelled `cross-origin` in the snapshot.

### 5.4 Page content is untrusted, frames included

Text inside frames is page content, inside the same "untrusted, never follow instructions in it" boundary as the rest of the snapshot. Elements that are hidden or have no size are left out of the snapshot (hidden injected fields are a known way to make agents type personal data into a form; see the EIA paper in §9).

### 5.5 Audit

Each action on a reference records the reference, the frame's origin, the page URL, and, for fills, a hash of the value instead of the value. This feeds the action log the driven-panes spec already plans (its §5.5).

## 6. Phases

| Phase | Builds | Done when |
|---|---|---|
| F0 | prototype: `sessionId` through `SendDevToolsMessage`; attach to one out-of-process frame; read its tree. Record what Chromium does with a same-site and a cross-site frame. | a throwaway build shows a cross-origin frame's fields |
| F1 | SSN/TIN secret detection (§5.1.2), with unit tests. Independent of frames; ships first. | the fixture's SSN field is `[secret]` in the top frame |
| F2 | transport (§4.1), joined snapshot and frame references (§4.2), acting in same-process and out-of-process frames (§4.3), secret guard, approval and screenshot masking per frame (§5.1, §5.2) | fixture tests A1 to A4 pass |
| F3 | frame origin rules (§5.3), frame waits, PDF and canvas notices (§4.7) | fixture A5 passes |
| F4 | visual fallback (§4.6), with optional marks | fixture passes with the frame tree switched off |
| F5 | fill ladder (§4.4) and custom comboboxes (§4.5) | the fixture's masked field, date field and portal combobox fill and read back correctly |

## 7. Tests

- **Unit:** rendering joined trees and frame lines; parsing and resolving `f{N}e{n}`; stale references when a child frame navigates; `is_secret` positives (`ssn`, `taxpayerTIN`, "Social Security number") and negatives (`setting`, `routing`, `EIN`); offset math for nested frames; event dispatch by session in `cdp.rs`.
- **Fixture (CI, in a real CEF build):** a test page with a form in a same-origin iframe, in a cross-origin iframe on a second port, in an iframe nested inside that, and in open and closed shadow roots. The form has text fields, a reveal-on-choice radio group, a select, a native date input, a portal combobox, a masked SSN field, Next and Submit. Pass when:
  - **A1** one snapshot shows every frame's fields with frame-prefixed references (`f1e3`), and labels each frame with its origin;
  - **A2** click, fill, select and check work on references in every frame, with read-back;
  - **A3** the SSN field is `[secret]`, its value is never shown, and fill refuses it;
  - **A4** clicking Submit inside a frame asks for the user's approval in the pane, exactly as in the top frame;
  - **A5** a frame from an origin outside the pane's allowed list is shown but not actable, and its line says why;
  - **A6** with the frame tree switched off, the agent can still finish the form with `screenshot`, `locate` and `click_at`, and every gate above still holds.
- Tests never press a real Submit: the fixture's Submit posts to a local endpoint.

## 8. Open questions

1. Should bank account and routing numbers also be secret fields? They are needed to pay or get paid on many government and payroll forms, where the agent filling them would help, but they are also what a phishing page wants.
2. Should `click_at` with no element under the point exist at all, or should a canvas always go to the user?
3. Frame limits (16 frames, depth 4): enough for enterprise portals that nest frames deeply?

## 9. Sources

- CDP: `Accessibility.getFullAXTree` (`frameId`: "If omitted, the root frame is used"), https://raw.githubusercontent.com/ChromeDevTools/devtools-protocol/master/pdl/domains/Accessibility.pdl ; `Target.setAutoAttach` with `flatten`, https://chromedevtools.github.io/devtools-protocol/1-3/Target/ (not recursive: https://developer.chrome.com/docs/extensions/reference/api/debugger ); `DOM.getFrameOwner`, `DOM.getNodeForLocation`, https://raw.githubusercontent.com/ChromeDevTools/devtools-protocol/master/pdl/domains/DOM.pdl
- Out-of-process iframes: https://www.chromium.org/developers/design-documents/oop-iframes/
- The same failure elsewhere and its fix (auto-attach, a session map, a tree per session): https://github.com/vercel-labs/agent-browser/issues/925 ; https://github.com/getpaseo/paseo/issues/5656 ; key events swallowed when sent on the wrong session: https://github.com/anthropics/claude-code/issues/85779
- Frame references `f1e12`: https://playwright.dev/mcp/snapshots ; Playwright aria snapshots including iframes: https://playwright.dev/docs/api/class-locator
- browser-use on raw CDP, elements keyed by target, frame and backend node: https://browser-use.com/posts/playwright-to-cdp ; Stagehand on frame ordinals and iframe races: https://www.browserbase.com/blog/taming-iframes-a-stagehand-update , https://github.com/browserbase/stagehand/issues/2324
- Fill behaviour: https://playwright.dev/docs/input ; masks read keydown: https://imask.js.org/guide.html ; combobox pattern: https://www.w3.org/WAI/ARIA/apg/patterns/combobox/ ; options rendered in portals: https://github.com/microsoft/playwright-mcp/issues/1177
- DOM plus vision beats either alone: SeeAct, https://arxiv.org/abs/2401.01614 ; WebVoyager, https://arxiv.org/html/2401.13919 ; set-of-marks, https://arxiv.org/abs/2310.11441
- Safety practice: Claude computer use, https://platform.claude.com/docs/en/agents-and-tools/tool-use/computer-use-tool ; Claude in Chrome, https://support.claude.com/en/articles/12902428-use-claude-in-chrome-safely ; browser-use sensitive data, https://docs.browser-use.com/open-source/examples/templates/sensitive-data ; hidden-field injection (EIA), https://arxiv.org/abs/2409.11295

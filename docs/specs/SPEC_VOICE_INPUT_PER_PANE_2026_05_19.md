# SPEC: Voice input — per-pane, header button near pane controls, Terminal + Agent

**Date:** 2026-05-19
**Author:** AgentX
**Status:** Draft
**Supersedes (live only in PR #741, not yet on main):**
- `specs/SPEC_VOICE_INPUT_2026_05_08.md` — original options analysis ([view in PR #741](https://github.com/agentmuxai/agentmux/pull/741/files))
- `specs/SPEC_VOICE_INPUT_IMPL_WEB_SPEECH_2026_05_08.md` — original impl spec (agent-pane-only, footer button)

**Salvages from:** [PR #741](https://github.com/agentmuxai/agentmux/pull/741) `feat: voice input via Web Speech API` (open, 11 days old, pre-dates ~145 commits of agent-pane churn)

---

## TL;DR

Voice-to-text input via the Web Speech API, scoped per pane via a mic button in the **top-right of the pane frame header, adjacent to the pane controls (maximize, close)**. Initially supported by the **Terminal** and **Agent** pane types. Click the button on the pane you want to speak into; the singleton recognition session redirects its output to that pane. Closing the button or clicking it on a different pane retargets.

---

## 1. What changes vs the original PR

| Dimension | Original PR #741 (2026-05-08) | This spec (2026-05-19) |
|---|---|---|
| **Activation location** | Mic button in `AgentFooter` (bottom of agent pane) | Mic button in `BlockFrame_Header` (top-right of any supporting pane, next to max/close) |
| **Supported panes** | Agent pane only | Terminal + Agent (extensible to others later) |
| **Pane targeting** | Last-focused agent pane | Pane whose mic button is *clicked* (explicit) |
| **Output injection** | Append to agent textarea + interim preview | **Agent:** same. **Terminal:** stream characters to the PTY via `controllerinput`. |
| **Hotkey** | Global `Ctrl+Shift+V` toggles on last-focused agent | Same hotkey, but toggles the **currently focused** pane (whichever supports voice) |
| **Singleton design** | ✓ one `webkitSpeechRecognition`, swappable target | ✓ same — re-use `useVoiceInput.ts` as-is |

The singleton + `registerPane(handle)` pattern in `useVoiceInput.ts` already supports multi-pane targeting cleanly — only the *placement* and *handle wiring* differ.

---

## 2. Scope & non-goals

### In scope (this initiative)

- Mic button in frame header for `view: "term"` and `view: "agent"` panes
- Singleton voice recognition session (already implemented, cherry-picked)
- Per-pane `PaneVoiceHandle` registration (already implemented)
- Visual feedback: pulse-ring on the active mic button; highlight on the receiving pane
- `Ctrl+Shift+V` toggles voice on the focused pane (no-op on non-supporting panes)
- Browser permission flow + graceful unavailability

### Out of scope (deferred)

- More pane types (Browser, Editor, Subagent, Drone) — these don't have a clear text-input target
- Server-side STT (Whisper, Vosk) — Web Speech API is the v1 path; server STT is a follow-up spec
- Voice *commands* (e.g., "send", "stop") — only transcript injection now
- Multi-language UI — `lang = navigator.language` is enough for v1
- Per-pane interim previews on the terminal (the terminal doesn't have a notion of "preview text" — see §6)

---

## 3. Architecture

### 3.1 Salvaged primitives (in this PR)

| File | Purpose | Source |
|---|---|---|
| `frontend/types/speech.d.ts` | Web Speech API type declarations | PR #741 (unchanged) |
| `frontend/app/hook/useVoiceInput.ts` | Singleton session + `registerPane(handle)` API | PR #741 (unchanged) |
| `frontend/app/element/MicButton.tsx` | Toggle icon button with pulse-ring active state | PR #741 (moved from `view/agent/components/` since it's now pane-agnostic) |
| `frontend/app/element/MicButton.scss` | Pulse-ring keyframes | Same move |

### 3.2 Integration points (next PR — see §7)

| Location | Change |
|---|---|
| `frontend/app/block/blockframe.tsx` | `BlockFrame_Header` renders `<MicButton />` in the right-side controls group (next to maximize/close) when `blockData.meta.view ∈ {"term", "agent"}`. Calls `voice.registerPane(handle)` when the button toggles on for this pane. |
| `frontend/app/view/term/termViewModel.ts` | Expose a `voiceHandle: PaneVoiceHandle` that calls `RpcApi.ControllerInputCommand` with the text as `inputdata64` — voice characters stream into the PTY. |
| `frontend/app/view/agent/agent-view.tsx` (or `useAgentStream.ts`) | Expose a `voiceHandle` that appends to the textarea (existing PR #741 logic, re-located from `AgentFooter`). |
| `frontend/app/store/keymodel.ts` | `Ctrl:Shift:v` resolves to "toggle voice on the focused pane" by looking up the focused block's view and calling its `voiceHandle`. |

---

## 4. The `PaneVoiceHandle` contract

```typescript
export interface PaneVoiceHandle {
    appendFinal: (text: string) => void;
    setInterim: (text: string) => void;
}
```

- `appendFinal(text)` — fired once when a phrase is finalized by the recognizer. Pane writes it to wherever its "typed input" goes.
- `setInterim(text)` — fired continuously as the user speaks. Pane shows a preview (if it has the affordance) or no-ops.

### Terminal pane implementation

```typescript
// termViewModel.ts
get voiceHandle(): PaneVoiceHandle {
    return {
        appendFinal: (text) => {
            // Stream each char as a controllerinput event so the PTY
            // sees natural typing. Honor xterm's input modes (echo,
            // line-buffered, etc.) by writing through the same path as
            // keyboard input.
            const bytes = new TextEncoder().encode(text);
            const inputdata64 = btoa(String.fromCharCode(...bytes));
            RpcApi.ControllerInputCommand(TabRpcClient, {
                blockid: this.blockId,
                inputdata64,
            });
        },
        setInterim: () => { /* no-op — terminal has no preview affordance */ },
    };
}
```

### Agent pane implementation

```typescript
// agent-view.tsx — building on PR #741's footer logic
const voiceHandle: PaneVoiceHandle = {
    appendFinal: (text) => {
        const ta = textareaRef.current!;
        ta.value = baseValue + text + " ";
        ta.dispatchEvent(new Event("input", { bubbles: true }));
        baseValue = ta.value;
    },
    setInterim: (text) => {
        const ta = textareaRef.current!;
        ta.value = baseValue + text;
        ta.dispatchEvent(new Event("input", { bubbles: true }));
    },
};
```

---

## 5. UX: where the button lives

The mic button lives in the **top-right of the frame header**, grouped with the existing pane controls (maximize, close), positioned just before them so the destructive close action remains at the corner. Rationale:

- Co-locates with the other pane-scoped controls so users learn one "pane chrome" region instead of two
- Active state (pulse-ring) is visible peripherally without competing with the pane content on the left side
- Click target is well-separated from the view icon / pane title hit area, so accidental activation while reading the header is unlikely
- Symmetric across all panes — terminal and agent panes use the same chrome, so the button slot is in the same place regardless of view

When listening:

- The mic button shows a pulse-ring animation (existing `MicButton.scss` keyframes)
- The pane's frame border gets a subtle highlight tint (`outline: 1px solid var(--accent-color); outline-offset: -2px;` — same accent the agent footer already uses)
- Clicking the mic on a *different* pane retargets — the old pane's pulse stops, the new pane's starts

---

## 6. Hotkey behavior

`Ctrl+Shift+V` toggles voice on the **currently focused** pane, *if* its view supports voice.

- If the focused pane is `term` or `agent` → toggle that pane's mic
- If the focused pane is anything else (browser, editor, etc.) → no-op, surface a toast "Voice input not supported on this pane"
- If no pane is focused → no-op (silent)

This avoids the original PR's "last-focused" ambiguity: the user always sees which pane the voice will go to (whichever they're currently in).

---

## 7. Implementation phases

### Phase 1 (this PR) — building blocks, no integration yet

- ✓ Cherry-pick `useVoiceInput.ts`, `speech.d.ts`, `MicButton.{tsx,scss}` from PR #741
- ✓ Move `MicButton.{tsx,scss}` to `frontend/app/element/` (pane-agnostic location)
- ✓ Write this spec
- Build green, no behavior change yet (the building blocks aren't called from anywhere)

### Phase 2 — frame-header integration

- Add `<MicButton />` slot to `BlockFrame_Header` (gated on view ∈ {term, agent})
- Add `voiceHandle` to `termViewModel.ts` and `agent-view.tsx`
- Wire `Ctrl+Shift+V` in `keymodel.ts`
- Add "currently receiving voice" highlight to the receiving pane's frame
- Test: speak into terminal pane → text appears at PTY prompt; speak into agent → text appears in textarea

### Phase 3 — polish

- Permission UX: first-launch prompt is browser-mediated; intercept "permission denied" and surface a "Voice unavailable — enable microphone in browser settings" toast
- Per-pane mic-button tooltip: "Speak into this terminal (Ctrl+Shift+V)" / "Speak into this agent (Ctrl+Shift+V)"
- Settings toggle: `voice:enabled` (default true) to fully hide the buttons globally
- Long-press / right-click: language picker (defer if not requested)

### Phase 4 (deferred) — server-side STT

- Replace `webkitSpeechRecognition` with an Anthropic/OpenAI/Whisper server-side path
- Same `getVoiceSession()` API, just a different implementation under the hood
- Better accuracy + works in non-Chromium browsers, at the cost of latency + API key + cost

---

## 8. Closing PR #741

PR #741 stays open until this spec's Phase 1 lands. Then:

- Comment on #741 linking to this spec
- Close PR #741 with note: "Cherry-picked the building blocks (useVoiceInput, speech.d.ts, MicButton, *moved to `frontend/app/element/`*) into [new PR]. The integration is being redesigned for multi-pane support (Terminal + Agent), top-right frame-header button placement (next to max/close) instead of agent footer. See SPEC_VOICE_INPUT_PER_PANE_2026_05_19.md for the new design."
- Author of #741 can be re-invited to drive Phase 2 if they want, or it's open work

---

## 9. Risk register

| Risk | Mitigation |
|---|---|
| Web Speech API is Chromium-only (no Firefox/Safari support) | Accepted for v1 — AgentMux is Chromium-based. Future server STT (§Phase 4) lifts this. |
| User pastes voice text into the wrong pane (clicked button A, focus moved to B) | Click *binds* the target, not focus. The button on pane A keeps the target until clicked elsewhere. Doc this clearly. |
| Mic permission denied → silent failure | `onerror` handler in `useVoiceInput.ts` already surfaces; add a UI toast. |
| Terminal pane: voice text might land mid-command and break shell parsing | Accepted — same risk exists for paste. User judgment. |
| Voice text contains shell metacharacters in terminal | Not our problem to sanitize — terminal input is supposed to be transparent. |
| Network glitch mid-utterance | `onend` already auto-restarts the session if `isListening` stays true. |
| Multiple AgentMux instances both listening | Each instance has its own JS context, its own recognition session, its own mic stream. Browser-mediated; no coordination needed. |

---

## 10. Effort estimate

| Phase | Effort |
|---|---|
| 1. Building blocks + spec (this PR) | ½ day |
| 2. Frame-header integration | 1 day |
| 3. Polish (permission UX, tooltips, settings toggle) | ½ day |
| 4. Server STT (deferred) | 3-5 days |
| **Total for Phases 1-3** | **~2 days** |

---

## 11. What the cherry-pick keeps from PR #741

Without re-reading the PR, the salvaged code is:

- `useVoiceInput.ts` — the entire singleton design. `registerPane` already supports the multi-pane retargeting we need.
- `speech.d.ts` — type declarations, untouched.
- `MicButton.tsx` — relocated to `frontend/app/element/MicButton.tsx`. Already pane-agnostic; uses `getVoiceSession()` which is the singleton.
- `MicButton.scss` — relocated. Pulse-ring keyframes carry over.

What we discard:

- `AgentFooter.tsx` changes — re-doing the integration via `BlockFrame_Header` instead
- `_pending-footer.scss` voice-active border — moves to a frame-level highlight class
- `keymodel.ts` global binding — needs different routing (focused pane vs last-focused agent)
- Both old specs — annotated as superseded; this doc replaces them

---

*End of spec.*

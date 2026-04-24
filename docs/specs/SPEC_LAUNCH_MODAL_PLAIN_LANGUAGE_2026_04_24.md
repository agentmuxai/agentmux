# Spec: Launch Modal Plain-Language Rewrite

**Date:** 2026-04-24
**Status:** Draft, ready to implement
**Owner:** AgentA
**Supersedes UX of:** `docs/specs/SPEC_AGENT_DEFINITIONS_MODAL_2026_04_23.md` §6
**Touches:** `frontend/app/view/agent/components/AgentLaunchModal.tsx`,
             `frontend/app/view/agent/styles/_launch-modal-body.scss`

---

## 1. Problem

The launch modal today mixes a clean primary task ("name this new
agent and start it") with devops/CLI terminology that bleeds in
from the backend layer:

| What the UI says | What the user has to decode |
|---|---|
| "Runtime: Host / Container" | "Is this an OS runtime thing? What's the difference for *me*?" |
| "Runs on your machine with your shell" | "I don't know what a shell is" |
| "Runs inside Docker/Podman — sandboxed" | "I don't know what Docker or Podman is; what's sandboxed mean?" |
| "Image: (placeholder `python:3.11-slim`)" | "What's an image? Do I need to pick one?" |
| "Working dir: `data/agents/my-agent-20260424-143220/`" | "Why am I seeing a filesystem path? Can I change it? Should I?" |
| "1–64 characters. Becomes part of the working directory." | "Working directory, again — *why am I being told this?*" |

These aren't factually wrong, but they push the user down a stack
they didn't come here to think about. A user wants to name a helper
and start it. They should not need to understand Docker, shells,
filesystem slugification rules, or working-directory lifecycles to
finish the task.

## 2. Goals

- **G1.** No devops jargon (`shell`, `Docker`, `Podman`, `image`,
  `working directory`, `sandboxed`, `runtime`) in the default view.
- **G2.** Each field answers a plain-English "what does this do?" in
  one sentence. No filesystem paths visible by default.
- **G3.** Power users can still override the container image and
  see the generated directory name, but it moves under an
  **Advanced** disclosure that stays closed by default.
- **G4.** No change to `LaunchOverrides` / backend contract. Pure
  UX-layer rewrite.
- **G5.** Stylelint-clean, zero visual regression for users who
  don't open Advanced, tsc clean, same `.agent-launch-modal-*`
  BEM prefix (SCSS partial already exists at
  `frontend/app/view/agent/styles/_launch-modal-body.scss`).

## 3. Non-goals

- No new backend fields, no new RPC, no new props.
- No change to the agent-definition picker (#524, #525), the
  import modal, or the delete-confirm modal.
- No translation / i18n — copy is English only for now.
- No addition of new icons or illustrations (keep the visual weight
  unchanged).

---

## 4. Design

### 4.1 Field-by-field copy rewrite

| Field | Today's label + hint | New label + hint |
|---|---|---|
| Name | **Name** — *1–64 characters. Becomes part of the working directory.* | **Give this agent a name** — *So you can tell it apart from others. 1–64 characters.* |
| Runtime option A | **Host** — *Runs on your machine with your shell.* | **On this computer** — *Fastest. The agent can read and change files on your machine.* |
| Runtime option B | **Container** — *Runs inside Docker/Podman — sandboxed.* | **In a safe sandbox** — *Slower to start, but the agent can't touch files outside its own workspace. Recommended for untrusted tasks.* |
| Runtime legend | **Runtime** | **Where should it run?** |
| Image (Advanced only) | **Image** — *Leave blank to use the default image.* | **Override sandbox base** — *Leave blank unless you know exactly which base image you need.* |
| Working-dir preview | Shown by default under "Working dir: &lt;path&gt;" | **Removed from default view.** Shown inside Advanced as "Its files will live in &lt;name&gt;" — just the directory name, no `data/agents/` prefix. |

### 4.2 Progressive disclosure — "Advanced" section

A collapsed `<details>` element (or custom disclosure) at the
bottom of the body, labelled **"Advanced options"**. When closed
(default), it adds zero visual noise. When open, it reveals:

- The container-image override input (only relevant when "In a
  safe sandbox" is selected — otherwise the field is hidden inside
  the expanded panel with a greyed-out "only applies to sandbox
  runtime" note).
- The directory-name preview, phrased as a plain English
  sentence, not as a file path.

Rationale: the two technical things a power user might legitimately
care about (image override + name-to-directory mapping) stay
accessible, but they no longer define the modal's first impression.

### 4.3 Copy on the launch button

Today: `"Launching…"` while pending. **No change** — already plain
language.

### 4.4 Catalog blurb (`catalog()?.popoverMarkdown`)

Line 107-110 of the current modal renders the catalog entry's
`popoverMarkdown` above the form. This copy lives in
`frontend/app/view/agent/defaults/cli-catalog.ts` and often
contains CLI-specific phrasing ("Anthropic's Claude Code CLI",
"Codex — OpenAI's CLI", etc.).

**Decision:** leave the catalog copy alone. It describes the
*product* the user chose (e.g. "Claude Code"), not a UI control
they need to understand to finish the task. It also serves as the
user's "am I launching the right thing?" confirmation and removing
it would hurt more than help.

If catalog copy mentions implementation details ("runs
`claude --verbose …`"), that's a cli-catalog.ts content bug, fixed
separately — not a launch-modal concern.

### 4.5 Visual structure

```
┌── Launch Claude Code ──────────────────┐
│  <catalog blurb>                       │
│                                        │
│  Give this agent a name                │
│  [___________________________]         │
│  So you can tell it apart. 1–64 chars. │
│                                        │
│  Where should it run?                  │
│  ( ) On this computer                  │
│      Fastest. Can read+change files.   │
│  ( ) In a safe sandbox                 │
│      Slower. Can't touch your files.   │
│                                        │
│  ▸ Advanced options                    │
│                                        │
│  [Cancel]                [Launch]      │
└────────────────────────────────────────┘
```

Opened Advanced:

```
│  ▾ Advanced options                    │
│    Override sandbox base               │
│    [_____________________]             │
│    Leave blank unless you know         │
│    exactly which base image you need.  │
│                                        │
│    Its files will live in              │
│    `my-helper-20260424-143220`         │
```

## 5. Implementation

### 5.1 TSX changes (`AgentLaunchModal.tsx`)

1. Rename user-facing strings per §4.1. Keep the `LaunchOverrides`
   interface and backend-facing property names unchanged.
2. Add a local `showAdvanced = createSignal(false)` and wrap the
   image input + directory preview in a `<details>` /
   custom-toggled block. A native `<details>` is fine — modal-v2
   handles focus trap correctly for any focusable descendants.
3. Move the `<Show when={previewDir()}>` block into Advanced.
   Change its render from `Working dir: data/agents/<slug>/` to
   `Its files will live in <slug>` (no filesystem path).
4. When Advanced is collapsed and runtime === "container", the
   container image still uses the catalog default — behaviour is
   identical to today for the 99% path.
5. Hide the container-image input when runtime === "host" (as
   today) but also when Advanced is closed; when Advanced is open
   and runtime is "host", render the input **disabled** with a
   one-line hint "Only applies to the sandbox runtime" so power
   users switching runtimes understand the relationship.

### 5.2 SCSS changes (`_launch-modal-body.scss`)

1. Add `.agent-launch-modal-advanced` styles for the `<details>`:
   collapsed header row, subtle chevron, body padding matching
   the other fields. Use existing spacing tokens — no new values.
2. Remove (or relax) any CSS that assumed the preview is always
   visible — the preview is now inside Advanced.
3. No token churn, no color changes; keep the visual weight
   identical at first glance.

### 5.3 Tests

No unit tests in scope for the modal today. Add none (keeps this
PR focused). Validation is manual + the automated build.

### 5.4 Validation checklist

- ✅ `task build:frontend` succeeds
- ✅ `npx tsc --noEmit` clean
- ✅ `npm run lint:scss` green
- ✅ Manual smoke (`task dev`):
  - Open a definition card → modal opens
  - Enter a name, Enter → submits (backend unchanged, no
    regression)
  - Toggle "Advanced" → reveals image override + directory
    preview
  - Switch runtime to "host" while Advanced is open → image
    field greys out with the "Only applies to sandbox" note
  - Resize window while modal open → backdrop still clips panes
    (PR #544 behaviour unchanged)

---

## 6. Risks & mitigations

| Risk | Mitigation |
|---|---|
| Users who were relying on seeing the working directory at a glance now need to click Advanced | Plain-English sentence replaces path; if anyone complains, we can surface the slug inline under the name field — cheap to reverse. |
| "On this computer" / "In a safe sandbox" reads marketing-y | Tested internally with CLAUDE.md guidance ("explain fields in general easy-to-understand terms"). If a power-user cohort prefers the technical labels, a future "advanced labels" setting could flip them globally. Not in scope. |
| Native `<details>` may not style consistently across CEF versions | Fallback: a `createSignal`-driven button + conditional render. Either is trivial; pick whichever styles cleanest. |
| Users confused about what "sandbox" means if they've never heard it | The hint says what it *does* ("can't touch files outside its own workspace") — no need to define the word. |

## 7. Non-goals (re-stated)

- No content-model overhaul. `LaunchOverrides`, the launch RPC,
  the working-dir resolution logic, and the catalog config all
  stay identical.
- No new popovers or tooltips (the spec-quoted §6.3 popover
  plan from `SPEC_AGENT_DEFINITIONS_MODAL_2026_04_23.md` is a
  separate expansion — this PR is strictly copy + disclosure).

## 8. Cross-references

- `docs/specs/SPEC_AGENT_DEFINITIONS_MODAL_2026_04_23.md` §6 —
  original spec this refines.
- `frontend/app/view/agent/components/AgentLaunchModal.tsx` —
  target component.
- `frontend/app/view/agent/styles/_launch-modal-body.scss` —
  target SCSS partial.
- `frontend/app/view/agent/defaults/cli-catalog.ts` — the catalog
  entries whose copy is out of scope for this spec but could
  benefit from a similar plain-language pass.

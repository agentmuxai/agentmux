# Session status — ABF/harness-model work, 2026-08-17

Working directory: `C:/amx-dev`, branch `main`. Written as a handoff snapshot,
not a spec — see the linked issues/PRs for authoritative detail.

## Issue #2594 — Harness/model decoupling & ABF portability: COMPLETE

All 9 delivery-plan checkboxes shipped and merged. Every site that read
`agent.provider` directly instead of resolving through the agent's bound ABF
bundle (the "gate vs. actual launch can disagree" bug class #2592 first
found) is now fixed:

| PR | Item |
|---|---|
| #2596 | `AgentLaunchModal.tsx` |
| #2607 | Three backend clone-sites (`template.rs` create-from-template/fork, `v1_templates.rs` promote) |
| #2608 | `AgentIdentityModal.tsx` — product decision (confirmed live with the human operator): removed the post-creation `model_vendor_base_url` edit surface rather than relaxing bundle immutability, since the bundle has no field to mirror that value into |
| #2609 | `AgentPicker.tsx` — 5 live sites fixed; one "cache invalidation" site turned out to be dead code and was deleted; one redundant duplicate read was folded away |
| #2610 | `AgentInstallModal.tsx` — the actual `InstallStartCommand` payload, not just display |
| #2612 | `bind-to-agent-menu.ts` + `agent-identity-links-panel.tsx` — kept `computeBindCandidates` pure/sync via an optional resolver param |
| #2615 | `exportagents` — last item; confirmed `importagents` was already internally consistent, only the export read needed fixing |

Every PR: read-before-edit, a regression test verified to fail against the
pre-fix code before trusting it, a changeset, full relevant test-suite runs,
independent `gh api` verification of any review/approval claim (several
arrived via jekt — all verified directly against GitHub, never trusted on
the jekt's word alone; one earlier jekt in this session falsely claimed a
fix commit didn't exist and was correctly dismissed after independent
verification).

Issue #2594 itself has not been closed — left open for the human operator to
close.

## Adjacent work — not done by this agent

- **Clamk / issue #2603** — "Agent identity/history persistence protocol"
  (conversation-history fragmentation across channel/identity UUIDs). Same
  storage layer as ABF but a distinct concern. PR #2602 (Step 1) merged.
  Reached out to coordinate before Clamk's Step 3 (splitting CREDENTIAL
  isolation from CONVERSATION HISTORY storage in `identities_dir()`) — no
  scope conflict found with #2594's now-complete work. No reply yet on the
  open coordination questions (does Step 3 change `identities_dir()`'s
  public contract; file overlap sequencing).
- **Issue #2024** — Armory Brain/Bundle tab, bundle-as-composition-primitive
  (v2). Explicitly sequenced to start only after #2594 landed cleanly.
  Untouched this session.

## In-progress — harness + model creation-flow UX (this session, not yet shipped)

User-requested usability work, confirmed direction via clarifying questions:

1. **Explanatory copy** for harness vs. model/vendor concepts in the
   "new agent" pane (`AgentPicker.tsx`) and the create-from-template modal —
   confirmed: short inline hint text in the modal, recommended placement.
2. **Model picker at creation time** — confirmed: keep the existing
   template-card grid (each card already implies a harness), add a model
   dropdown once a card is picked, reusing the `getProvider(harnessId)?.models`
   pattern `AgentRuntimeDropup.tsx` already uses for the in-session runtime
   switcher.
3. **Backend wrinkle found mid-implementation, now blocking**: the obvious
   plan (add a `model` field to `agentdefcreatefromtemplate`'s RPC, write it
   into the new bundle) collides with existing semantics —
   `Memory.model` is **already** used to store the VENDOR label
   (`"anthropic"`/`"openai"`/`"google"`/`"custom"`, via
   `resolve_effective_vendor`), not an actual model choice like `"opus"` or
   `"gpt-5.5"`. The real per-session model picker
   (`AgentRuntimeDropup`/`AgentComposerStrip`) writes into **per-launch block
   runtime config** instead (`applyRuntimeChange`), not the bundle at all —
   there is no existing persisted "default model for this agent" field.
   **Awaiting a decision from the human operator**: (a) add a real new
   bundle column (e.g. `default_model`) distinct from the vendor-labeled
   `model` column — schema/migration work — or (b) have the creation-time
   picker just seed the first launch's runtime config, matching how model
   choice actually flows today, with a durable per-agent default left as a
   separate follow-up.

No code has been committed for this feature yet pending that decision.

## Also found and fixed this session (uncommitted)

- **Pane tab-strip overlay bug**: `.agent-picker` (the "new agent" pane
  content) had a flat `padding: var(--space-4)` with no extra top clearance
  for the floating `PaneTabStrip`, which renders even over the blank/picker
  tab (`agent-view.tsx`). Icons/header content rendered behind the strip.
  Fixed in `frontend/app/view/agent/styles/_picker.scss` by mirroring the
  exact `padding-top: calc(... + var(--pane-tab-strip-height, 28px))`
  pattern `_document.scss` already uses for `.agent-document`.
  **Status: fixed locally, uncommitted** — no changeset/PR yet.
- **Dev-session Vite lifecycle issue** (environment, not app code): running
  `task dev` via a monitored background shell got killed by the harness's
  own bashwrap idle-output timeout once the GUI window stopped producing
  stdout, which also killed the backgrounded Vite dev server while the
  window (protected by the launcher's own Job Object) stayed open —
  producing exactly the "icons turned into squares" symptom (Font Awesome
  webfonts/CSS 404ing once Vite died). Not a code fix; documented here in
  case it recurs. Workaround used: launch `task dev` fully detached via
  `cmd.exe /c start`, outside the harness's monitored process tree.

## Open items for the human operator

1. Harness+model creation-flow: pick (a) new bundle column vs. (b) seed
   first-launch runtime config (see above).
2. Whether to close issue #2594 now that all delivery items are merged.
3. `.agent-picker` padding fix: commit + changeset + PR, or fold into
   whichever PR ships the harness+model creation-flow work.

# SPEC: Agent launch name — no autofill, ghost-text placeholder only

**Status:** Implemented. **Supersedes:** [`SPEC_DEFAULT_AGENT_NAME`](SPEC_DEFAULT_AGENT_NAME.md) (GH issue #780).

## Summary

The agent-name field in `AgentLaunchModal` currently pre-fills itself with
`<Provider> Agent` (e.g. `Claude Agent`, `Claude Agent 2`) the moment the
launch form opens, so a user who clicks Launch without typing anything gets
an agent named after its model rather than its purpose. Reverting that:
the field opens empty, and its placeholder is a static ghost-text hint —
`Descriptive nickname` — instead of the model's display name. A name must
be typed to launch.

## Why revert #780

#780's own stated tradeoff was "friction with no payoff for the common
case." In practice the opposite turned out to matter more: a fleet of
agents all named after their model (`Claude Agent`, `Claude Agent 2`,
`Claude Agent 3`, ...) carries no information about which agent does what,
and the whole point of naming an agent — telling instances apart at a
glance in the picker, the tab strip, `DiscoverAgents`, `SendMessage`
targets — is defeated by a name that's just the provider repeated back.
Requiring a real name up front is a one-time three-second cost that pays
for itself on every subsequent glance at a multi-agent workspace.

## Decision

1. **Remove the autofill.** Delete the `createEffect` in
   `AgentLaunchModal.tsx` that called `defaultAgentName(displayName(),
   existing)` to populate the field on open. The field starts empty in New
   mode, exactly as it does today in Continue mode (which was never
   autofilled — it already carries the continued agent's real name).
2. **Static placeholder.** The input's `placeholder` changes from
   `displayName()` (the provider/model name — `"Claude"`, `"Codex"`, ...)
   to the literal string `"Descriptive nickname"`. Ghost text, not a
   default value: it's never committed unless the user types over it, so
   `hasName()` (which gates the Launch button — see `AgentLaunchModal.tsx`
   `hasName`/`disabled` wiring) correctly stays `false` until real input
   arrives.
3. **Provider-agnostic by construction, not by special-casing.**
   `AgentLaunchModal` is the single shared launch surface for every
   provider (`displayName()` already resolves generically from
   `catalog()?.displayName ?? props.agent.name`; there is no
   provider-specific fork of this form). Removing the effect in this one
   place is the complete fix — no per-provider changes are needed or
   exist to make.
4. **Dead code removal.** `defaultAgentName` and `cleanProviderLabel`
   (`app/view/agent/defaults/default-agent-name.ts`) had exactly one
   caller — the effect this spec removes. Deleted the module and its test
   file (`default-agent-name.test.ts`) rather than leave an unused export
   behind.

## Out of scope

- **Continue mode** is unaffected — it never autofilled (the name comes
  from the continued row via `handleContinueSelect`'s carry-over) and
  keeps its own "Agent name" label and disabled input.
- **Collision-suffix logic** (`Claude Agent 2`, `3`, ...) is deleted along
  with `defaultAgentName`, not repurposed — there is nothing left to
  suffix once there's no default value to collide against. Manually typed
  names still go through the existing `slugifyInstanceName` /
  `buildInstanceSlug` validation and duplicate-name handling elsewhere in
  the modal, unchanged.
- Issue #779 (the zombie-HWND-eats-keystrokes bug #780 cited as a reason
  to avoid requiring typing) is a separate, still-open problem. This spec
  reintroduces the friction #780 removed to work around it; #779 itself
  needs its own fix regardless of this decision.

## Files touched

- `frontend/app/view/agent/components/AgentLaunchModal.tsx` — remove the
  autofill `createEffect` and its `defaultNameApplied` flag; change
  `placeholder={displayName()}` to `placeholder="Descriptive nickname"`;
  drop the now-unused `defaultAgentName` import.
- `frontend/app/view/agent/defaults/default-agent-name.ts` — deleted.
- `frontend/app/view/agent/defaults/default-agent-name.test.ts` — deleted.

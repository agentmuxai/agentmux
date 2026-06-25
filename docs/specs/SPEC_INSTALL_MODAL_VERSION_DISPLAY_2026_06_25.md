# SPEC: Show CLI Version in Agent Install Modal

**Date:** 2026-06-25
**Status:** Draft
**Scope:** CSS-only + two JSX lines

---

## Goal

When the install modal opens for an agent CLI (e.g. Claude Code), display the
version that will be installed so the user knows exactly what they're getting
before clicking "Install now".

---

## Current state

`AgentInstallModalPanel` (`frontend/app/view/agent/components/AgentInstallModal.tsx`)
currently shows:

```
[icon]  Install Claude Code
        not installed — click below to install
```

The version is available inside the component (`provider()?.pinnedVersion`) but
never rendered. It is silently passed to `RpcApi.InstallStartCommand` as
`pinnedVersion` and appended to the npm package arg by the backend
(`@anthropic-ai/claude-code@2.1.185`). The user sees it only in the raw npm
output that scrolls through the xterm terminal — too late and too noisy.

---

## Data source

`pinnedVersion` comes from `ProviderDefinition` (`providers/index.ts:44–106`),
populated at build time per provider:

| Provider | Value |
|---|---|
| Claude Code | `"2.1.185"` (pinned) |
| Codex | `"0.116.0"` (pinned) |
| Gemini | `"0.32.1"` (pinned) |
| OpenClaw / Copilot / Pi | `"latest"` |

Access path inside the modal (already computed on line 57):
```typescript
const provider = () => getProvider(props.agent.provider);
const version  = () => provider()?.pinnedVersion;
```

No new API call, no network fetch — the value is synchronous.

---

## Design

### Placement

Display the version as a small badge immediately after the display name in the
modal header, on the same line:

```
[icon]  Install Claude Code  [v2.1.185]
        not installed — click below to install
```

Or, for `"latest"`:
```
[icon]  Install OpenClaw  [latest]
        not installed — click below to install
```

A badge (pill) is preferred over plain text so it reads as metadata rather than
part of the title.

### States

Version badge is visible in **all modal states**:
- `idle` (before install starts) ✓
- `installing` (elapsed timer visible) ✓
- `done` ✓
- `error` ✓

The version does not change mid-install, so a static reactive read is sufficient.

### "latest" label

When `pinnedVersion === "latest"`, show the badge text `"latest"` verbatim.
The actual resolved version will appear in the npm output in the terminal below —
no need to pre-resolve it via the npm registry.

---

## Implementation

### `AgentInstallModal.tsx` — header section

**Locate the header title element** (around line 354–365). It currently renders:

```tsx
<div class="install-modal-title">
    <img ... />
    <span class="install-modal-name">{displayName()}</span>
</div>
```

**Change to:**

```tsx
<div class="install-modal-title">
    <img ... />
    <span class="install-modal-name">{displayName()}</span>
    <Show when={version()}>
        <span class="install-modal-version">v{version()}</span>
    </Show>
</div>
```

Note: when `version()` is `"latest"`, the `v` prefix would read `"vlatest"` —
handle this case:

```tsx
<Show when={version()}>
    <span class="install-modal-version">
        {version() === "latest" ? "latest" : `v${version()}`}
    </span>
</Show>
```

Add the `version` accessor near the existing `displayName` accessor:

```typescript
const version = () => provider()?.pinnedVersion;
```

### SCSS — `_install-modal.scss` (or wherever install modal styles live)

```scss
.install-modal-version {
    display: inline-flex;
    align-items: center;
    padding: 1px 7px;
    border-radius: 10px;
    font-size: 11px;
    font-weight: 500;
    letter-spacing: 0.03em;
    background: color-mix(in oklab, var(--accent-color) 15%, var(--block-bg-solid-color));
    color: var(--accent-color);
    vertical-align: middle;
    margin-left: 6px;
    user-select: none;
}
```

Uses `--accent-color` and `--block-bg-solid-color` — the same oklab mixing
approach used for user-input bubbles, guaranteed to read correctly on any theme.

---

## Files to change

| File | Change |
|---|---|
| `frontend/app/view/agent/components/AgentInstallModal.tsx` | +1 `const version` accessor; +3 JSX lines (`<Show>` + `<span>`) |
| `frontend/app/view/agent/styles/_install-modal.scss` (or colocated) | +10 lines new `.install-modal-version` rule |

No backend changes needed — version is already available on the frontend at
modal open time.

---

## Open questions

1. **Exact header JSX location**: The explore agent found the header around lines
   354–365 but the file is 459 lines. Read the actual header block before editing
   to confirm the selector.

2. **SCSS file location**: Confirm whether install modal styles live in
   `_install-modal.scss`, inline in `AgentInstallModal.tsx` via a `.scss` import,
   or in another partial. Find and use the existing file.

3. **Should `"latest"` resolve to an actual version?** Out of scope for this PR —
   the npm output in the terminal shows the resolved version. A future PR could
   call `npm view <pkg> version` to pre-resolve.

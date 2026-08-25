# SPEC: Surface `~/.claude/CLAUDE.md` (read-only) in Global Memory

**Date:** 2026-08-24
**Status:** proposed. **Note:** the file this spec targets in §1-§4 turned
out to be the wrong one — see §5 for the corrected design (the CLAUDE.md
at AgentMux's shared Claude provider config dir, not the ambient
`~/.claude/CLAUDE.md`). Filename kept as-is since it's already referenced
by the PR/commits; read §5 for what actually shipped.

---

## 0. Ask

> right ... but do you use the system claude.md even though you run inside
> of agentmux? / right .. so that needs to appear, the global one, as well
> as all the others, including their paths.

Followed by a scoping question, answered: show **only** the machine-global
`~/.claude/CLAUDE.md` file — its real path plus a content preview,
read-only. (Example per-provider paths and a live per-agent path listing
were both explicitly declined for this pass.)

---

## 1. Current behavior (audited against source, 2026-08-24)

Confirmed by directly reading this agent's own startup files earlier in the
same conversation: Claude Code loads `~/.claude/CLAUDE.md` as a global,
user-level config file, independent of any per-project `CLAUDE.md` and
independent of AgentMux entirely — it's Claude Code's own convention, not
something AgentMux writes or manages.

`SPEC_GLOBAL_MEMORY_SYSTEM_TIER_2026_08_24.md` §1 describes "a working,
informally 'highest priority' instructions file" at a *different* path
(`~/.agentmux/agents/CLAUDE.md`) and states it has "no representation in
the Armory UI at all." Re-verified in this conversation: that exact path
**does not exist on this host** — `ls ~/.agentmux/agents/CLAUDE.md` →
`No such file or directory`. The real file matching that spec's own
description (hand-maintained, no UI, no audit trail, actually loaded) is
`~/.claude/CLAUDE.md`. That spec's factual claim about the path was wrong
for this deployment (stale or imprecise, not re-verified here); this spec
doesn't attempt to fix that document, just surfaces the file that's
actually real.

No RPC command reads this file today. No frontend code references its path
or content anywhere in the Armory UI.

---

## 2. Design

### 2.1 Backend: one new read-only RPC command

```rust
// rpc_types/commands.rs, alongside the memory-bundle commands
pub const COMMAND_GET_CLAUDE_GLOBAL_CONFIG: &str = "getclaudeglobalconfig";
```

```rust
// agent_handlers/memory.rs — registered alongside the other memory commands
engine.register_handler(
    COMMAND_GET_CLAUDE_GLOBAL_CONFIG,
    Box::new(move |_data, _ctx| {
        Box::pin(async move {
            let path = crate::backend::base::get_home_dir().join(".claude").join("CLAUDE.md");
            let (content, exists) = match std::fs::read_to_string(&path) {
                Ok(c) => (Some(c), true),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => (None, false),
                Err(e) => return Err(format!("getclaudeglobalconfig: {e}")),
            };
            Ok(Some(json!({
                "path": path.to_string_lossy(),
                "content": content,
                "exists": exists,
            })))
        })
    }),
);
```

No parameters — the path is fixed, not a caller-supplied argument (no
path-traversal surface: this reads exactly one hardcoded, well-known
location, nothing else). Read-only: no write/delete counterpart, matching
the scoping decision (§0) that this is display-only, not an editing
surface. A non-`NotFound` I/O error (e.g. permission denied) surfaces as an
RPC error rather than being silently swallowed — distinct from "file
doesn't exist," which is a normal, expected state on a host where the user
never created one.

### 2.2 Frontend: one new RPC wrapper + one new atom

```ts
// frontend/app/store/rpc-api/memory.ts, alongside ListMemoriesCommand
GetClaudeGlobalConfigCommand(
    client: RpcClient,
    data: Record<string, never> = {},
    opts?: RpcOpts,
): Promise<{ path: string; content: string | null; exists: boolean }> {
    return client.rpcCall("getclaudeglobalconfig", data, opts);
},
```

`GlobalBrainViewModel` gains:

```ts
private _claudeGlobalConfig = createSignal<{ path: string; content: string | null; exists: boolean } | null>(null);
claudeGlobalConfigAtom: Accessor<{ path: string; content: string | null; exists: boolean } | null> = this._claudeGlobalConfig[0];
```

Fetched once in the constructor alongside the existing `refresh()` call —
a static, machine-wide fact, not something that needs re-fetching on every
`memories:changed` event the way the DB-backed sections do (nothing in
this app can currently *write* to this file, so there's no event to react
to; a manual page reload picks up an out-of-band hand-edit, same as any
other read-once-at-mount display).

### 2.3 UI: a distinct, clearly-labeled, read-only block

Rendered in `global-brain-manager.tsx`, **above** the system tier (it's
the thing the system tier was conceptually meant to replace/complement —
seeing it first gives the operator the full picture before the "here's
where you'd add AgentMux's own policy" section):

```tsx
<Show when={model.claudeGlobalConfigAtom()}>
    {(cfg) => (
        <div class="global-brain-machine-config">
            <div class="global-brain-machine-config-header">
                <span class="global-brain-machine-config-badge" title="Hand-maintained on disk, not managed by AgentMux, shared by every agent on this machine">
                    Machine-wide (Claude Code)
                </span>
                <code class="global-brain-machine-config-path">{cfg().path}</code>
            </div>
            <Show
                when={cfg().exists}
                fallback={<p class="global-brain-machine-config-empty">No file at this path yet.</p>}
            >
                <pre class="global-brain-machine-config-content">{cfg().content}</pre>
            </Show>
        </div>
    )}
</Show>
```

Read-only by construction — no textarea, no save button, no edit affordance
of any kind. The badge tooltip states the "why" (hand-maintained, not
AgentMux's, shared machine-wide) so a human doesn't wonder why this one
block behaves differently from every editable section around it.

`exists: false` (no file at that path on this host) renders a plain
"No file at this path yet" message rather than hiding the block entirely —
the path itself, and the fact that Claude Code would pick up a file there
if one existed, is useful information even when empty.

---

## 3. Out of scope

- Editing this file through the UI — §0's scoping decision, read-only only.
- Migrating/copying this content into the system tier — a separate,
  explicit action a human would take deliberately (copy-paste from this
  new preview into "+ Add AgentMux system entry"), not automated here.
- Any equivalent "global config" file for other providers (Codex, Gemini,
  etc.) — out of scope, not researched, no evidence such a thing exists
  for them the way `~/.claude/CLAUDE.md` does for Claude Code.
- Live-updating the preview if the file changes on disk while the pane is
  open (no filesystem watcher) — matches this being a read-once-at-mount
  display, §2.2.
- Fixing `SPEC_GLOBAL_MEMORY_SYSTEM_TIER_2026_08_24.md`'s own incorrect
  path claim — noted in §1, not corrected in that document by this spec.

---

## 4. Test plan

**Rust:**
- [ ] `getclaudeglobalconfig` against a `HOME` pointed at a tempdir with a
      real `.claude/CLAUDE.md` present: returns `{path, content: Some(...),
      exists: true}` with the exact file content.
- [ ] Same, but no `.claude/CLAUDE.md` file: `{path, content: None, exists:
      false}`, not an error.
- [ ] Returns an error (not a silent empty result) for a genuine I/O error
      other than not-found (simulate via an unreadable file, if the test
      harness can construct one cross-platform; otherwise document as
      manually-verified only).

**Frontend:**
- [ ] `GlobalBrainViewModel.claudeGlobalConfigAtom()` populates after
      construction from a mocked `GetClaudeGlobalConfigCommand`.
- [ ] Component renders the path and content when `exists: true`; renders
      the "No file at this path yet" fallback when `exists: false`; renders
      nothing (no block at all) while the atom is still `null`
      (pre-fetch).

**Manual (`task dev`):**
- [ ] Confirm the block shows this machine's real shared-provider-config
      `CLAUDE.md` path and content (see §5 — not the ambient
      `~/.claude/CLAUDE.md`), positioned above the system tier.

---

## 5. Post-review revision (2026-08-24, PR #2794 — codex P1)

**The original design (§1-§4 above) was wrong about which file a spawned
Claude agent actually loads as its "global" config.** Codex caught it,
citing `agent_open.rs:303-312` and
`SPEC_PROVIDER_ISOLATION_2026_06_20.md` §5b directly:

- Every AgentMux-spawned Claude agent gets `CLAUDE_CONFIG_DIR` set to an
  AgentMux-owned directory (`agent_open.rs`'s `auth_dir` — by default
  `DataPaths::provider_auth_dir("claude")`, i.e.
  `~/.agentmux/shared/providers/claude`; identity-bound agents use a
  separate per-identity dir instead).
- `SPEC_PROVIDER_ISOLATION_2026_06_20.md` §5b, already "confirmed on disk"
  before this spec existed: *"`CLAUDE_CONFIG_DIR` relocates the **entire**
  Claude home... User `CLAUDE.md` → `<CLAUDE_CONFIG_DIR>/CLAUDE.md`."*
- So the ambient `~/.claude/CLAUDE.md` this spec originally targeted is
  simply **not** what a `CLAUDE_CONFIG_DIR`-isolated spawned agent reads —
  displaying it under "shared by every agent on this machine" was false
  for the common case, not just imprecise.

**Re-verified empirically (2026-08-24), which also resolved an apparent
contradiction:** this agent's own `CLAUDE_CONFIG_DIR` is a per-identity
directory (`~/.agentmux/channels/.../identities/<uuid>/claude`) with no
`CLAUDE.md` in it at all — and neither does the default shared dir
(`~/.agentmux/shared/providers/claude/`). So on this host, as of this
date, **no AgentMux-spawned Claude agent has a populated global
`CLAUDE.md` anywhere** — the only real, populated file matching that
description is the ambient `~/.claude/CLAUDE.md` this spec originally
found, which (per the isolation architecture) isn't actually in the read
path for a normal spawned agent. Whatever separate mechanism surfaced that
file's content into *this* agent's own context earlier in the conversation
that prompted this spec is not resolved here — out of scope, a distinct
question from what this UI block should display.

**Fix:** resolve the path via the exact same logic `agent_open.rs` uses
for the DEFAULT (non-identity-bound) case —
`DataPaths::provider_auth_dir("claude")`, falling back to
`~/.agentmux/shared/providers/claude` when `DataPaths::from_env()` fails,
mirroring `agent_open.rs`'s own fallback verbatim. This is the file that
actually IS shared across every default-provider Claude agent on this
machine, closing the gap between the block's claim and its behavior for
the common case. Identity-bound agents (a separate per-identity dir) are
still not covered — flagged, not fixed, same "known gap, not blocking"
treatment §3's out-of-scope items already use elsewhere in this spec chain.

Badge copy changed from "Machine-wide (Claude Code)" (implying universal
truth) to "Claude Code — shared provider config" (accurately scoped, with
the identity-bound caveat moved into the tooltip) to match.

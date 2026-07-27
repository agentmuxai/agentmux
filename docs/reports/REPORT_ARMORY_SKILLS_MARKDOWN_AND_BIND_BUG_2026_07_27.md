# Analysis: Armory Skills — plain-text content rendering + broken "Bind to agent"

**Date:** 2026-07-27
**Author:** Agent3
**Verified against:** `main` @ `38978e6ba`, live-verified against the running `task dev` build (branch `agent3/agent-armory-rename-stash`, no relevant divergence from `main` for this area).
**Status:** Analysis — root causes confirmed (one live-verified via CDP), fix not yet implemented.
**Scope:** `frontend/app/view/skill/skill-manager.tsx` (Armory's Skills tab) + its backend, `agentmux-srv/src/server/app_api/skill.rs`.

## User's request (verbatim, for traceability)

> lets shift to the armory pane .. first, get the Content of the Skills parts to be rendered in markdown (currently it is just plain text) .. also, binding to an agent is not working, fix that too .. first write analysis to file

## 1. Skill content renders as plain text, not Markdown

`frontend/app/view/skill/skill-manager.tsx:75-84` renders `description`, `trigger`, and `content` as raw `<pre>` text nodes — no Markdown processing:

```tsx
<Show when={skill().description}>
    <span class="agent-primitive-modal-field-label">Description</span>
    <pre class="agent-primitive-modal-field-value">{skill().description}</pre>
</Show>
<Show when={skill().trigger}>
    <span class="agent-primitive-modal-field-label">Trigger</span>
    <pre class="agent-primitive-modal-field-value">{skill().trigger}</pre>
</Show>
<span class="agent-primitive-modal-field-label">Content</span>
<pre class="agent-primitive-modal-field-value">{skill().content || "(none)"}</pre>
```

Confirmed live (screenshot below) — the "Systematic Debugging" skill's `content` field is genuine Markdown (`# Systematic Debugging`, `**bold**`, numbered lists, a code-style trigger) rendered completely unstyled.

**Fix: reuse the existing `Markdown` component** (`frontend/app/element/markdown.tsx`) — a `unified`/`remark-gfm`/`rehype-sanitize` pipeline already used throughout the app (agent chat messages via `MarkdownBlock.tsx`, the editor, tool-result overlays), not a new dependency. Standard invocation:

```tsx
<Markdown text={skill().content || "(none)"} scrollable={false} />
```

`scrollable={false}` matches `MarkdownBlock.tsx`'s own usage — the Skills detail pane already has its own scroll container (`.agent-primitive-modal-detail` / `PrimitiveListDetail`), so `Markdown`'s own internal scroll wrapper would double up otherwise.

**Scope decision — `content` only, not `description`/`trigger`.** `description` and `trigger` are short, single-line-ish free text (a UI label + a trigger phrase, per the create/edit form's plain `<input type="text">` fields, `skill-manager.tsx:147-152` and `:138-145`) — there's no evidence they're meant to hold Markdown, and the create form doesn't offer a Markdown-authoring affordance for them (single-line inputs, not a textarea). Only `content` is authored via a multi-line `<textarea>` (`skill-manager.tsx:154-160`) and is the field the example skill's real body demonstrates is genuine prose Markdown. Recommend leaving `description`/`trigger` as `<pre>` (or even a plain `<span>`, since they're single-line) and converting only `content`.

**Two render sites need this fix, not one.** `frontend/app/view/agent/components/AgentSkillsModal.tsx:107-116` (the per-agent Skills tab, opened via the Stash modal) has the byte-for-byte identical `<pre>`-for-`content` pattern — it's effectively a duplicated detail view, not a shared component. Both need the same `<Markdown>` swap; there's no single shared "skill detail" component to fix once. (Extracting one is a reasonable follow-up but out of scope for this fix — matching the existing duplication rather than doing an unrelated refactor alongside a bug fix.)

The Brain/Memories manager (`global-brain-manager.tsx:199-200`) has the same `<pre>`-for-user-content pattern independently — noted for awareness, but out of scope (not part of this request).

## 2. "Bind to agent" is completely non-functional — live-verified

### 2.1 Reproduction

Live in the running dev build: Armory → Skills → select a skill → pick an agent in "Bind to agent" → click **Bind**. Result, verbatim, rendered in the error banner:

> **Bind failed: FORBIDDEN: unauthenticated agent connection**

This happens for **every** agent, **every** skill — it is not data-dependent. Screenshot evidence and the exact CDP-driven repro steps are in this investigation's working notes; the failure string above was captured directly from the live DOM after a scripted click, not inferred.

### 2.2 Root cause

The call chain:

```
skill-manager.tsx:100        onClick → model.bindToAgent(skill().id, model.bindAgentIdAtom())
skill-model.ts:112           RpcApi.SkillBindCommand(TabRpcClient, { agent_id, skill_id })
rpc-api/skill.ts:47-53       client.rpcCall("skill.bind", data, opts)
skill.rs:182-215             register_skill_bind → check_s1(&ctx, &req.agent_id)?   ← fails here
```

`check_s1` (`agentmux-srv/src/server/app_api/mod.rs`) requires the **calling WebSocket connection itself** to be authenticated as the same agent named in the request:

```rust
pub(super) fn check_s1(ctx: &RpcContext, req_agent_id: &str) -> Result<(), String> {
    if ctx.agent_id.is_empty() {
        return Err("FORBIDDEN: unauthenticated agent connection".to_string());
    }
    if ctx.agent_id != req_agent_id {
        return Err("FORBIDDEN: agent_id mismatch".to_string());
    }
    Ok(())
}
```

`ctx.agent_id` is stamped exactly once per connection, only when that connection sends a `bus:register` frame (`agentmux-srv/src/server/websocket.rs:373-380`) — something only an actual agent CLI process does on its own out-of-band connection. The Armory pane's WebSocket (`TabRpcClient` / the shared per-window `globalWS`, `frontend/app/store/rpc-util.ts`) is the ordinary human/CEF dashboard connection and **never** sends `bus:register` — grepped the entire `frontend/` tree, zero matches outside a manual test harness script. So `ctx.agent_id` is permanently empty for every RPC call the Armory ever makes, and `check_s1` rejects the bind request before `wstore.skill_bind(...)` (the actual DB write, `skill.rs:209`) is ever reached.

**This is a design/wiring gap, not a data bug.** No DB schema mismatch, no wrong column, no swallowed error — the write path (`Store::skill_bind`, `skills.rs:355-363`, `INSERT OR IGNORE INTO db_agent_skills_ref`) and the read-back path (`skill_list_global`'s `bound_count`) both correctly agree on parameter names and would work fine *if the write ever ran*.

### 2.3 Why this exists — `skill.bind` was built for a different caller

`skill.rs`'s own header comment says the plain (non-catalog) commands are "Agent-scoped" — i.e. `skill.bind`/`skill.unbind` were designed for an **agent to bind a skill to itself**, over its own authenticated MCP/agent connection (the same trust model the catalog commands explicitly opt out of: *"skill.catalog.*: … no `check_s1` (mirrors bundle.* auth shape, since the Armory has no agent connection context to gate on")*). No REST route or `agentmux-mcp` tool wraps `skill.bind` either (grepped `agentmux-mcp/src/main.rs` and the REST router — no matches), so as designed, `skill.bind` was likely never reachable from anywhere except a hand-authenticated test harness.

The Armory's catalog-side "Bind to agent" UI (button, `bindToAgent()`, the agent-picker dropdown) was added by a later PR extending the catalog surface (`skill-model.ts:62`'s own comment: `// Catalog-side bind action (#1960 gap #3)`) — but it was wired to call the pre-existing agent-scoped `skill.bind` command instead of a catalog-safe equivalent. The gate that makes sense for "an agent binds a skill to itself" was never relaxed (or forked) for "a human clicks Bind in the dashboard" — so the feature has been unreachable since the button was added.

**The identical bug affects MCP Servers' bind, by the same construction** (`mcp.rs`'s `register_mcp_bind` calls the same `check_s1` at the same call depth, added and catalog-wired in the same two PRs as Skills) — flagged here for awareness since it's the same root cause, but **out of scope for this fix**; the user asked specifically about Skills. Worth a follow-up once this pattern is proven out.

### 2.4 Recommended fix

Add a **catalog-scoped bind command**, mirroring the existing `skill.catalog.list/upsert/delete` trust tier exactly (no `agent_id` context to check, no `check_s1`) rather than loosening the existing `skill.bind`'s `check_s1` gate:

- `agentmux-srv/src/backend/rpc_types/commands.rs`: new `COMMAND_SKILL_CATALOG_BIND = "skill.catalog.bind"`.
- `agentmux-srv/src/server/app_api/skill.rs`: new `register_skill_catalog_bind`, same handler body as `register_skill_bind` minus the `check_s1` call (keep the existing "only global skills, or already-bound ones, may be bound" safety check — that's an ownership/data-integrity rule, unrelated to the auth gate being removed).
- `frontend/app/store/rpc-api/skill.ts`: new `SkillCatalogBindCommand`.
- `frontend/app/view/skill/skill-model.ts:112`: `bindToAgent` calls the new command instead of `SkillBindCommand`.

**Why a new command instead of just removing `check_s1` from `skill.bind`:** `skill.bind` may still be the intended future surface for an agent binding a skill to *itself* (self-service), and its `check_s1` gate (same-agent-only) is a real, meaningful restriction for that caller — removing it would silently widen what an authenticated agent connection can do (bind arbitrary skills to *other* agent ids), a security-relevant change nobody asked for. A catalog-tier sibling command is the same pattern already established for list/upsert/delete, costs one small mirrored handler, and leaves the existing command's contract untouched.

**Unbind:** the Armory's Skills catalog view currently has no "Unbind" affordance at all (confirmed reading `skill-manager.tsx` — only a Bind button and dropdown, no per-agent bound-list to unbind from) — so there is nothing to fix on the unbind side to make the currently-exposed UI work. Not adding one here; flagged as a possible small follow-up, not part of "binding is not working."

## 3. Suggested implementation order

1. Backend: `commands.rs` constant + `skill.rs` handler (small, additive, no existing behavior changed).
2. Frontend: `rpc-api/skill.ts` command wrapper + `skill-model.ts` call-site swap.
3. Frontend: `skill-manager.tsx` + `AgentSkillsModal.tsx` — `<pre>` → `<Markdown text={...} scrollable={false} />` for `content` only, in both files.
4. Verify live: re-run the exact CDP repro from §2.1 and confirm Bind now succeeds and `bound_count`/"Used by N agents" updates; visually confirm Markdown renders (headings, bold, numbered list) in both the Armory and per-agent Skills detail views.

## File/line reference table

| Concern | File | Line(s) |
|---|---|---|
| Armory Skills detail view — plain-text render | `frontend/app/view/skill/skill-manager.tsx` | 75-84 |
| Per-agent Skills detail view — same bug | `frontend/app/view/agent/components/AgentSkillsModal.tsx` | 107-116 |
| Markdown component to reuse | `frontend/app/element/markdown.tsx` | 67-75 (props), exported component |
| Bind button / call site | `frontend/app/view/skill/skill-manager.tsx` | 97-103 |
| `bindToAgent` view-model method | `frontend/app/view/skill/skill-model.ts` | 105-117 |
| `SkillBindCommand` RPC wrapper | `frontend/app/store/rpc-api/skill.ts` | 47-53 |
| `register_skill_bind` handler (the `check_s1` failure site) | `agentmux-srv/src/server/app_api/skill.rs` | 182-215 |
| `check_s1` definition | `agentmux-srv/src/server/app_api/mod.rs` | ~856-864 |
| `bus:register` — the only thing that ever sets `ctx.agent_id` | `agentmux-srv/src/server/websocket.rs` | 373-380 |
| Catalog commands' existing no-`check_s1` precedent | `agentmux-srv/src/server/app_api/skill.rs` | 237-351 (list/upsert/delete) |
| DB write (confirmed correct, never reached) | `agentmux-srv/src/backend/storage/skills.rs` | 355-363 |
| MCP Servers has the identical bug (not in scope here) | `agentmux-srv/src/server/app_api/mcp.rs` | ~190-223 |

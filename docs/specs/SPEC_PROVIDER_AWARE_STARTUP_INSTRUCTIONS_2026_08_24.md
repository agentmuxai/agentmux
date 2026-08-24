# SPEC: Provider-aware startup instructions filename + visibility in Global Memory

**Date:** 2026-08-24
**Status:** proposed
**Builds on:** `docs/specs/SPEC_GLOBAL_MEMORY_SYSTEM_TIER_2026_08_24.md` (system
tier, implemented same day) and
`docs/specs/SPEC_ABF_V0_2_PROVIDER_AWARE_COMPONENTS_AND_NATIVE_MEMORY_2026_08_10.md`
(flagged this exact gap without fixing it).

---

## 0. Ask

> Fix the provider-aware naming so Codex and Gemini agents use
> provider-specific files .. we also need to hook in all the applicable
> files into the global memory so users can view them.

Immediate prior context in the same conversation: a Global Memory UI wording
pass removed "CLAUDE.md" from user-facing copy after confirming every
provider's agent gets a file literally named `CLAUDE.md` written to its
working directory today, regardless of provider — then the user asked what
files their own (Claude-provider) agent instance actually reads at startup,
which surfaced this gap directly.

---

## 1. Current behavior (audited against source, 2026-08-24)

### 1.1 The hardcode

`agentmux-srv/src/backend/agent_config.rs`'s `build_config_files()` (the
function that actually materializes instructions into a launched agent's
working directory) takes no provider parameter and unconditionally writes
`AgentConfigFile { filename: "CLAUDE.md".to_string(), .. }` (line 114) for
every provider. `frontend/app/view/agent/agent-config-builder.ts`'s
`buildConfigFiles()` mirrors this exactly — `files.push({ path: "CLAUDE.md",
... })` (line 81), also provider-blind.

Both are already flagged (not fixed) in
`SPEC_ABF_V0_2_PROVIDER_AWARE_COMPONENTS_AND_NATIVE_MEMORY_2026_08_10.md`
§1.2: *"A Codex or Gemini CLI agent gets a `CLAUDE.md` written to its
working directory today, not the harness's own native convention."*

### 1.2 The one live caller, and what's already in scope at that call site

Rust: the only non-test call site is `agentmux-srv/src/server/app_api/agent_open.rs:729`,
inside `write_agent_config_files`. By that point in the function,
`agent.provider` has already been reassigned (line 176) to
`app_state.id_store.resolve_effective_provider_id(&agent)` — the same
resolution that decides which harness actually gets spawned (accounts for a
bound ABF bundle's own provider overriding a possibly-drifted
`agent.provider` column; see `resolve_effective_provider_id`,
`agents.rs:1087`). So the effective provider ID is already sitting in
`agent.provider` at the `build_config_files` call site — no new resolution
needed, just threading the existing value through.

Frontend: `agent-model.ts:531`'s call to `buildConfigFiles(contentMap,
skills, agent, instanceName)` sits inside `launchAgentDefinition`, after
`const effectiveProvider = await resolveEffectiveLaunchProvider(agent)` (line
344) and `const provider = PROVIDERS[effectiveProvider] ?? ...` (line 346) —
the resolved `ProviderDefinition` object is already in scope, used two lines
later for `checkNodejsForProvider(provider.id)`. Same story: the value
exists, it just isn't passed down.

### 1.3 `instructions_by_provider` exists in the schema but is dead weight

`SPEC_ABF_V0_2_PROVIDER_AWARE_COMPONENTS_AND_NATIVE_MEMORY_2026_08_10.md`
added `db_bundles.instructions_by_provider` (JSON `{provider_id: content}`)
for ABF *export/import* portability. Confirmed via grep: every
non-test construction site (`agent_open.rs:1077`, `agent_seed.rs:274`) sets
it to the empty-object placeholder `"{}"` and nothing ever reads it back for
*runtime* config-file composition — it's wired into `bundle_export.rs`/
`bundle_import.rs` only. **Out of scope here** (§5) — this spec fixes the
*filename* every provider's agent gets, not per-provider *content*
divergence. The column stays exactly as unused-at-runtime as it is today;
a future spec could wire it in once there's an actual authored use case.

### 1.4 `.claude/`-namespaced files are NOT part of this spec

`build_config_files`/`buildConfigFiles` also write `.claude/commands/<trigger>.md`
(skills-as-slash-commands), `.claude/skills/<slug>/SKILL.md` (Agent
Skills format), `.claude/settings.json` (hooks), and `.mcp.json`. All four
are Claude-Code-specific extension mechanisms with no established
cross-provider equivalent researched here (Codex's own skill/instruction
extension model, if any, is a separate research effort; `.mcp.json`'s shape
is itself an ecosystem convention Codex/Gemini/etc. increasingly share, so
touching it isn't obviously beneficial the way the instructions-file rename
is). **Out of scope** (§5) — this spec touches only the single "Soul +
AgentMD + Memory + Skills index" instructions file `build_config_files`
already treats as one dedicated concept (the `claude_md_parts`/`claudeMdParts`
block, lines 77-117 / 55-82).

---

## 2. Research: confirmed native startup-instructions filename per provider

Verified against each provider's own current public documentation
(2026-08-24), not guessed — matching this codebase's own discipline for
`ProviderConfig.base_url_env_var` ("set only where independently verified,
not guessed", `providers.rs:95-99`). Confidence noted per row; anything
short of "confirmed" is called out explicitly rather than presented as
equally solid.

| Provider | Filename | Confidence | Source / reasoning |
|---|---|---|---|
| `claude` | `CLAUDE.md` | confirmed | Existing, extensively verified throughout this codebase (this agent's own working directory, e.g.). |
| `codex` | `AGENTS.md` | confirmed | `SPEC_CODEX_PROVIDER_INTEGRATION_2026_08_08.md` §10.2: "Codex's native project instruction discovery continues to load user/repository `AGENTS.md` files normally." |
| `gemini` | `GEMINI.md` | confirmed | [Gemini CLI docs](https://geminicli.github.io/gemini-cli/docs/cli/gemini-md/): context files default to `GEMINI.md`; `AGENTS.md` support exists but requires an explicit `contextFileName` setting override, so `GEMINI.md` is the correct *default*-behavior target. |
| `qwen` | `QWEN.md` | confirmed | [Qwen Code settings docs](https://qwenlm.github.io/qwen-code-docs/en/users/configuration/settings/): `QWEN.md` is a built-in default `contextFileName`; `AGENTS.md` needs explicit config (open feature requests [#2006](https://github.com/QwenLM/qwen-code/issues/2006), [#504](https://github.com/QwenLM/qwen-code/issues/504) ask for it to become default — confirming it isn't yet). |
| `copilot` | `AGENTS.md` | confirmed | [GitHub Copilot CLI custom-instructions docs](https://docs.github.com/en/copilot/how-tos/copilot-cli/customize-copilot/add-custom-instructions): supports `AGENTS.md` (root, single-file, no subdirectory needed) alongside `.github/copilot-instructions.md`, `CLAUDE.md`, `GEMINI.md` — `AGENTS.md` chosen as the one canonical target since Copilot has no single privileged default the way Gemini/Qwen do. |
| `openclaw` | `AGENTS.md` | confirmed convention, **unconfirmed path** | [OpenClaw AGENTS.md reference](https://docs.openclaw.ai/reference/AGENTS.default): `AGENTS.md` is a required bootstrap file OpenClaw itself creates, read from each agent's workspace at session start. Exact on-disk path is OpenClaw's own Gateway-daemon-managed sandbox (`/sandbox/.openclaw/workspace/` per public examples) — whether that maps 1:1 onto AgentMux's `working_directory` for an ACP-bridged `openclaw acp` session is **not independently verified here**. Implemented as best-effort root-level `AGENTS.md`; flagged as a known gap in §6, not blocking this spec. |
| `pi` | `.pi/APPEND_SYSTEM.md` | confirmed | [pi-coding-agent npm docs](https://www.npmjs.com/package/@mariozechner/pi-coding-agent): `.pi/SYSTEM.md` *replaces* pi's default system prompt; `.pi/APPEND_SYSTEM.md` *appends* to it. AgentMux's Soul+AgentMD+Memory content is additive background, not a full system-prompt replacement (pi's own default prompt carries pi's own tool-usage instructions) — `APPEND_SYSTEM.md` is the correct target, not `SYSTEM.md`. |
| `antigravity` | `GEMINI.md` | **inferred, not independently doc-confirmed** | Antigravity CLI's own settings live at `~/.gemini/antigravity-cli/settings.json` — same `~/.gemini/` namespace root as Gemini CLI itself, consistent with `providers.rs`'s own doc comment ("Emits the same stream-json NDJSON envelope as Gemini CLI (its sibling harness)"). No explicit Antigravity docs page confirms `GEMINI.md` context-file behavior independently of that shared-lineage inference. Flagged in §6. |
| `muxcode` | `CLAUDE.md` | confirmed by design intent | AgentMux's own first-party CLI; `providers.rs`'s own doc comment for `MUX_CODE` explicitly states it "emits claude-compatible stream-json NDJSON... ClaudeTranslator handles it without modification" — a deliberate compatibility choice by the same team, not a guess. |
| `kimi` | *(none — no file written)* | confirmed absence | `docs/specs/KIMI_PROVIDER_INTEGRATION_SPEC.md` §(system prompt injection) already researched this and found no auto-read markdown convention: *"Does Kimi read a system prompt file like CLAUDE.md? Answer: Unknown... does not appear to auto-read CLAUDE.md... For Phase 1, skip KIMI.md generation."* Re-verified 2026-08-24: Kimi CLI's only file-based prompt customization is `--agent-file <yaml>` with a `system_prompt_path` field — an explicit CLI flag AgentMux doesn't pass in `KIMI.launch_args` (`providers.rs:304-311`) — and an open, unshipped feature request ([kimi-cli#1856](https://github.com/MoonshotAI/kimi-cli/issues/1856)) for a project-level `system_prompt.md` override. Writing *any* markdown file for Kimi today would be inert output nobody reads; this spec keeps the prior decision (skip) rather than inventing a filename. |

---

## 3. Design

### 3.1 New field: `ProviderConfig.startup_instructions_filename` (Rust) / `ProviderDefinition.startupInstructionsFilename` (TS)

```rust
// providers.rs
pub struct ProviderConfig {
    // ...existing fields...
    /// Path (relative to the agent's working directory) this provider
    /// natively auto-discovers its startup instructions from — e.g.
    /// `"CLAUDE.md"`, `"AGENTS.md"`, `".pi/APPEND_SYSTEM.md"`. `None` when
    /// no native file-based convention is confirmed to exist (currently
    /// only `kimi` — see docs/specs/SPEC_PROVIDER_AWARE_STARTUP_INSTRUCTIONS_2026_08_24.md
    /// §2); `build_config_files` skips writing the instructions file
    /// entirely in that case rather than writing inert content.
    pub startup_instructions_filename: Option<&'static str>,
}
```

Mirrored on the frontend catalog (`providers/types.ts`):

```ts
export interface ProviderDefinition {
    // ...existing fields...
    /** Mirrors Rust's ProviderConfig.startup_instructions_filename — path
     *  (relative to the agent's working directory) this provider natively
     *  auto-discovers its startup instructions from. `undefined` when no
     *  native file-based convention is confirmed (currently only `kimi`). */
    startupInstructionsFilename?: string;
}
```

Every one of the 10 entries in `providers.rs`'s static registry and
`providers/catalog.ts`'s `PROVIDERS` map gets the value from §2's table
added explicitly — no `Default` impl / optional-with-fallback shortcut, same
discipline `base_url_env_var` already established ("a provider added to one
without the other is exactly the bug class this function exists to close" —
`buildRuntimeArgs.ts:141-144`'s own reasoning for `providerSupportsModelFlag`,
reused here). A new provider added later without this field is a compile
error (Rust: struct-literal field is required, not `Option` with a
`Default`) / a straightforward, greppable omission (TS: optional field, but
`providerSupportsModelFlag`-style helper functions below make its absence
observable rather than silently falling back to `"CLAUDE.md"`).

### 3.2 `build_config_files` / `buildConfigFiles` take the resolved provider ID

```rust
pub fn build_config_files(
    content_map: &HashMap<String, String>,
    skills: &[AgentSkill],
    agent_name: &str,
    agent_id: &str,
    agent_slug: &str,
    working_directory: &str,
    provider_id: &str,               // NEW
) -> Vec<AgentConfigFile> {
    // ...
    let instructions_filename = providers::get_provider(provider_id)
        .and_then(|p| p.startup_instructions_filename);
    if let Some(filename) = instructions_filename {
        if !claude_md_parts.is_empty() {
            files.push(AgentConfigFile { filename: filename.to_string(), content: ... });
        }
    }
    // unknown provider_id (shouldn't happen — caller already validated it
    // to spawn the harness at all) falls through the same way an unset
    // `startup_instructions_filename` does: no instructions file written,
    // rather than guessing "CLAUDE.md" for something unrecognized.
}
```

Unknown-provider handling is deliberately the same no-op path as
`kimi`'s "no file" case — never a silent `"CLAUDE.md"` fallback, which
would just reintroduce this spec's own bug under a different trigger
(a provider ID typo or a not-yet-registered ID producing a file the
provider in question was never confirmed to read).

Frontend mirror takes the analogous `providerId?: string` parameter,
looked up via `PROVIDERS[providerId]?.startupInstructionsFilename` (with
the same `resolveProviderAlias` fallback `getProvider`/`PROVIDERS` lookups
already use elsewhere in this file, e.g. `agent-model.ts:346`).

### 3.3 Call-site changes

- `agent_open.rs:729`: pass `&agent.provider` (already the
  effective-resolved value per §1.2) as the new final argument. This path's
  own global-memory-bundle injection (§1.2, `agent_open.rs:706-720`) already
  happens at the `content_map["memory"]` level, BEFORE `build_config_files`
  resolves a filename — already filename-agnostic, no change needed there.
- `agent-model.ts:531`: pass `provider.id` (the already-resolved
  `ProviderDefinition`, in scope from line 346) as the new final argument.
- **`editor_handlers.rs`'s `WriteAgentConfig` handler (the actual "click
  Launch" path) — a second hardcode, found while implementing, not caught
  by the original research pass.** `CommandWriteAgentConfigData` is just
  `{working_dir, files}` (`rpc_types/block.rs:303`) — no provider ID
  travels over this RPC at all, since the frontend already fully resolves
  file contents before sending. Its write loop (lines 305-348) gates BOTH
  global-memory-bundle injection (`inject_global_bundles`) AND
  ownership-aware materialization (`write_claude_md_respecting_ownership`)
  behind a literal `if file.path == "CLAUDE.md"`. Left as-is, a
  Codex-provider agent's now-correctly-named `AGENTS.md` would silently
  fall through to the generic unconditional write with **no** Global
  Memory content injected at all — a real functional regression this spec
  must not introduce. **Fix:** added `providers::is_known_startup_instructions_filename(path)`
  (membership check against the registry's `startup_instructions_filename`
  values — single source of truth, no parallel list to drift) and used it
  to extend `inject_global_bundles` (which is content-shape-based, not
  filename-specific — it looks for generic `# Memory`/`# Available Skills`
  markers already present regardless of target filename, so it's safe to
  reuse verbatim) to every recognized startup-instructions file, not just
  literal `"CLAUDE.md"`. Ownership protection (`write_claude_md_respecting_ownership`)
  is **not** extended — see §5.
- Every test call site in `agent_config.rs` (§1.2's ~15 sites, all currently
  passing 6 positional args) gets a 7th `"claude"` argument (preserves
  existing test intent — they're testing skill/hook/mcp materialization,
  not provider-filename resolution specifically) **except** the new tests
  in §7 that exist specifically to exercise other providers.
- `agent-config-builder.test.ts`'s existing `buildConfigFiles(...)` call
  sites: same treatment — add the new param only where a test's *point* is
  provider-filename behavior; leave others on the default (`undefined` →
  falls back identically to `"claude"`'s `CLAUDE.md`, so this is a
  non-breaking addition for every unrelated existing test).

### 3.4 Global Memory UI: show every applicable file, not just one

`frontend/app/view/brain/global-brain-manager.tsx`'s preview section
(recently changed to auto-expand and drop hardcoded "CLAUDE.md" wording —
same conversation, immediately prior turn) currently renders one preview
block with no indication of *which* file(s) the content actually becomes.
Once §3.1-§3.3 land, that's actively incomplete: the same Global Memory
content is now genuinely destined for up to 8 different filenames (9 minus
kimi, which gets none) depending on which providers are in use in this
workspace.

**Design: a small "Applies to" summary above the preview**, grouping
providers by resolved filename (since multiple providers share a filename —
`claude`/`muxcode` → `CLAUDE.md`; `gemini`/`antigravity` → `GEMINI.md`), plus
an explicit callout for `kimi` (the one provider that gets nothing). Content
itself stays a single preview block underneath — it does NOT diverge per
provider today (§1.3: `instructions_by_provider` isn't wired into runtime
composition), so rendering N identical content blocks would be redundant,
not informative. What's missing today is *visibility into where this one
block of content actually lands*, which is what "hook in all the applicable
files ... so users can view them" asks for.

```tsx
// global-brain-manager.tsx, above the existing preview <div>
<div class="global-brain-applies-to">
    <span class="global-brain-applies-to-label">Applies to:</span>
    <For each={model.filenameGroupsAtom()}>
        {(group) => (
            <span class="global-brain-applies-to-chip" title={group.providerNames.join(", ")}>
                <code>{group.filename}</code>
            </span>
        )}
    </For>
    <Show when={model.noFileProvidersAtom().length > 0}>
        <span class="global-brain-applies-to-chip global-brain-applies-to-chip-warning"
              title={`${model.noFileProvidersAtom().join(", ")}: no confirmed startup-instructions file — see SPEC_PROVIDER_AWARE_STARTUP_INSTRUCTIONS_2026_08_24.md §2`}>
            not yet applied to: {model.noFileProvidersAtom().join(", ")}
        </span>
    </Show>
</div>
```

`GlobalBrainViewModel` gains two derived accessors (both plain `createMemo`s
over the static provider catalog — no new RPC, no new backend surface):

```ts
/** Providers grouped by resolved startup-instructions filename, e.g.
 *  [{filename: "CLAUDE.md", providerNames: ["Claude", "Mux Code"]}, ...].
 *  Static — derived from the PROVIDERS catalog, not per-workspace agent
 *  data (every provider's file gets this content regardless of whether
 *  a workspace currently has an agent using it — the whole point is
 *  telling the operator up front, not only after they've launched one). */
filenameGroupsAtom: Accessor<{ filename: string; providerNames: string[] }[]>;
/** Providers with no confirmed startup-instructions file (currently just
 *  Kimi) — surfaced as an explicit "not applied to" callout rather than
 *  silently omitted, so a user isn't left wondering why a Kimi agent
 *  doesn't see their Global Memory content. */
noFileProvidersAtom: Accessor<string[]>;
```

Preview toggle label also changes from the single "Combined startup
instructions preview" (this conversation's prior turn) to reference the
grouping, e.g. "Combined preview (same content, every applicable file)" —
exact copy is an implementation-time judgment call, not load-bearing design.

### 3.5 Ownership guard for non-`CLAUDE.md` files (codex P1, PR #2788)

**Found during review, not in the original design pass.** §3.3's initial
implementation wrote every recognized non-`CLAUDE.md` startup-instructions
file via a plain, unconditional `std::fs::write` — the same path every
other config file (`.mcp.json`, skill files) already used. Codex correctly
flagged this as a genuine, novel data-loss regression: **before** this
spec, every provider's agent got `CLAUDE.md` written regardless of
provider — wrong, but harmless to a real Codex/Gemini/etc. project, since
AgentMux was never writing to the filename that project's own real
`AGENTS.md`/`GEMINI.md` actually lived at. Once the filename resolution
became CORRECT per provider (§3.1-§3.3), an unconditional write would
silently destroy a pre-existing, user-authored project file the moment its
name collided with the now-correctly-resolved target.

**Fix:** `write_startup_instructions_respecting_existing` — a new function
mirroring `write_claude_md_respecting_ownership`'s OWNED-vs-foreign marker
check (a new, generic `STARTUP_INSTRUCTIONS_MANAGED_MARKER` first-line
comment, since `AGENTS.md`/`GEMINI.md`/`QWEN.md`/pi's `APPEND_SYSTEM.md`
don't each need distinct marker text — an HTML comment renders invisibly
in every markdown viewer and every one of these providers reads its
instructions file as plain text fed into a prompt, so a leading comment
line is universally harmless regardless of provider): if AgentMux wrote
the file (fresh, or on a prior launch — detected via the marker), freely
regenerate it; otherwise, never touch it, and this agent's Soul/AgentMD/
Memory content is simply not delivered via that file for this launch
(logged, not silently dropped).

**Deliberately NOT replicated:** `write_claude_md_respecting_ownership`'s
`@import`-line side-file fallback for the foreign case (write to a side
file, offer an importable reference so a foreign file's owner can still
opt in). That mechanism is Claude-Code-`@import`-syntax-specific;
Copilot's own docs do confirm the identical `@relative/path` include
syntax works inside `AGENTS.md` too, but whether Codex/Gemini/Qwen/pi's
own harnesses recognize an equivalent directive in their own native files
is unverified per-provider research this spec didn't do. Landing the
simpler "freely regenerate if ours, never touch if foreign" guarantee now,
without inventing an unverified per-provider include mechanism, is
strictly safer than shipping the unconditional-write it replaces — and
strictly safer than blocking this whole spec on that research. Revisit as
a follow-up once each provider's own include syntax is confirmed (§5).

Wired into both live write paths (`agent_open.rs`'s `write_agent_config_files`
and `editor_handlers.rs`'s `WriteAgentConfig` handler), gated the same way
as §3.3's global-memory-bundle-injection extension: via
`providers::is_known_startup_instructions_filename(path)` membership
against the registry, not a literal filename list to keep in sync by hand.

---

## 4. Resolved design decisions

1. **Filename source of truth — resolved: one field on `ProviderConfig`/
   `ProviderDefinition`, populated from independently-verified public docs
   per provider (§2), not guessed, not left to fall back to `"CLAUDE.md"`.**
   Matches this codebase's existing `base_url_env_var` discipline.
2. **`kimi` gets no instructions file — resolved: `None`, not a placeholder
   filename.** Writing an unread file is strictly worse than writing
   nothing: it implies a working feature that doesn't exist. Re-affirms the
   prior `KIMI_PROVIDER_INTEGRATION_SPEC.md` Phase-1 decision rather than
   silently reversing it.
3. **`instructions_by_provider` (content divergence per provider) — resolved:
   out of scope, untouched.** This spec fixes *where* the (single, shared)
   content lands, not *whether* the content itself should differ by
   provider. Conflating the two would block this fix on a much larger,
   separately-scoped feature.
4. **`.claude/`-namespaced files (skills, hooks, `.mcp.json`) — resolved:
   out of scope, untouched.** No cross-provider equivalent researched;
   renaming those without equivalent research would be worse than leaving
   them as-is (silently wrong for non-Claude providers in a new way, rather
   than the current, at-least-consistent-across-providers behavior).
5. **Global Memory preview — resolved: one content block, N filename
   labels, not N content blocks.** Content doesn't actually diverge per
   provider (per #3 above); duplicating the preview text per filename would
   misleadingly suggest it does.
6. **Ownership protection for non-Claude startup-instructions files —
   resolved: a simpler marker-based exists-guard (§3.5), not the full
   `write_claude_md_respecting_ownership` mechanism.** Originally landed as
   "not extended, explicitly flagged" — codex P1 on PR #2788 correctly
   caught that as a genuine data-loss regression (an unconditional write
   could now destroy a pre-existing, user-authored `AGENTS.md`/`GEMINI.md`/
   etc., something the old always-`CLAUDE.md` behavior never risked, since
   it was never writing to the filename real projects actually used).
   `write_claude_md_respecting_ownership`'s full mechanism (marker check
   PLUS an `@import`-line side-file fallback for the foreign case) is
   deeply coupled to Claude Code's own `@import` syntax — a syntax with no
   confirmed equivalent in AGENTS.md/GEMINI.md/QWEN.md conventions, so
   replicating THAT part remains deferred (§5). But the marker-check half
   (freely regenerate what we own, never touch what we don't) needs no
   per-provider syntax knowledge at all — an HTML comment as a leading
   line is universally harmless plain text regardless of which harness
   reads the file — so there was no reason to defer it too. Global-memory-
   bundle injection IS extended to every recognized startup-instructions
   filename (§3.3) — that part is filename-agnostic and safe to reuse
   as-is.

---

## 5. Out of scope

- Per-provider content divergence (`instructions_by_provider` wiring into
  `build_config_files`) — §4.3.
- Renaming/translating `.claude/commands/`, `.claude/skills/`,
  `.claude/settings.json`, `.mcp.json` per provider — §4.4.
- Resolving OpenClaw's exact ACP-Gateway-sandboxed working-directory mapping
  (§2's "unconfirmed path" row) — flagged, not blocked on.
- Independently confirming Antigravity's `GEMINI.md` inference beyond the
  shared-`~/.gemini/`-namespace circumstantial evidence (§2) — flagged, not
  blocked on.
- Adding `--agent-file`/`system_prompt_path` support to actually deliver
  content to Kimi via its own (non-file-auto-discovery) mechanism — a
  materially different, larger change (alters `KIMI.launch_args`, requires
  synthesizing a YAML agent-file, is a new content-delivery *mechanism* not
  a filename fix) than this spec's scope. Real follow-up if ever prioritized.
- Extending `write_claude_md_respecting_ownership`'s `@import`-line
  side-file fallback (the foreign-file recovery path, not the OWNED-vs-
  foreign check itself — that part IS implemented, §3.5) to non-Claude
  startup-instructions files — §4.6. A foreign `AGENTS.md`/`GEMINI.md`/
  etc. is safely left untouched (§3.5), but unlike `CLAUDE.md`'s foreign
  case, this agent's content isn't offered via any importable side file
  either — it's simply not delivered for this launch. Needs a
  per-provider import-syntax answer this spec didn't research.
- A per-agent (as opposed to per-provider-class) override of the resolved
  filename — no stated need; every provider of a given class already gets
  a single, correct, non-configurable target.

---

## 6. Known gaps carried forward (flagged, not blocking)

- OpenClaw's exact working-directory mapping under its ACP Gateway sandbox
  (§2).
- Antigravity's `GEMINI.md` inference is circumstantial (shared config
  namespace + shared NDJSON schema with Gemini CLI), not an independent
  doc citation (§2).

Both are pre-existing unknowns this spec surfaces rather than introduces —
today's behavior for both providers is unconditionally wrong (`CLAUDE.md`,
which neither reads by any confirmed convention), so landing a best-effort,
explicitly-flagged filename is strictly an improvement even where the exact
path isn't 100% nailed down.

---

## 7. Test plan

**Rust** (`providers.rs`, `agent_config.rs`):
- [ ] Every provider in the registry has a `startup_instructions_filename`
      matching §2's table exactly (one assertion per provider, mirroring
      the existing `every_provider_declares_at_least_one_supported_vendor`
      pattern).
- [ ] `kimi`'s `startup_instructions_filename` is `None`.
- [ ] `build_config_files` with `provider_id: "codex"` produces a file at
      `AGENTS.md`, not `CLAUDE.md`.
- [ ] `build_config_files` with `provider_id: "gemini"` produces
      `GEMINI.md`; `"qwen"` → `QWEN.md`; `"pi"` → `.pi/APPEND_SYSTEM.md`;
      `"claude"` / `"muxcode"` → `CLAUDE.md` (both, unchanged from today).
- [ ] `build_config_files` with `provider_id: "kimi"` produces **no**
      instructions file at all — `files.iter().find(|f| ...)` for any of
      the known filenames returns `None`, and (if `content_map` has
      soul/agentmd/memory content) that content is silently NOT written
      anywhere, confirmed by the file count not increasing versus a
      content_map-only-mcp/hooks baseline.
- [ ] `build_config_files` with an unrecognized `provider_id` (e.g.
      `"not-a-real-provider"`) behaves identically to `kimi` — no
      instructions file, no panic.
- [ ] Existing skill/hook/mcp materialization tests (§1.2's ~15 sites)
      still pass unmodified in behavior once given an explicit `"claude"`
      7th argument — regression guard that the new parameter doesn't
      change output for the already-covered claude-provider path.
- [ ] `write_startup_instructions_respecting_existing` (§3.5): fresh
      working dir writes directly with the marker; a marker-prefixed
      (AgentMux-owned) file regenerates cleanly across multiple calls with
      no accumulation; a foreign file (with or without prior content, incl.
      an empty file) is never touched, byte for byte; a non-UTF-8 foreign
      file is treated as foreign, not as absent; nested paths (`.pi/APPEND_SYSTEM.md`)
      get their parent directory created on a fresh dir.

**Frontend** (`agent-config-builder.test.ts`, `providers/*.test.ts`):
- [ ] `PROVIDERS` catalog: same per-provider filename assertions as the
      Rust registry test, kept in sync (mirrors the existing
      `pin-consistency.test.ts` cross-language-sync pattern).
- [ ] `buildConfigFiles` with `providerId: "codex"` → `path: "AGENTS.md"`;
      same per-provider matrix as the Rust test.
- [ ] `buildConfigFiles` with `providerId: "kimi"` → no instructions file
      in the returned array.
- [ ] `buildConfigFiles` with no `providerId` (omitted) → `CLAUDE.md`,
      unchanged — confirms the addition is non-breaking for every
      pre-existing call site that doesn't pass the new param.

**Global Memory UI** (`global-brain-model.test.ts`):
- [ ] `filenameGroupsAtom` groups `claude`+`muxcode` under `CLAUDE.md`,
      `gemini`+`antigravity` under `GEMINI.md`, etc., matching §2's table.
- [ ] `noFileProvidersAtom` contains exactly `["Kimi"]` (display name, not
      `"kimi"` the ID) given the current catalog.

**Manual** (`task dev`):
- [ ] Launch a Codex-provider agent (if credentials available); inspect its
      working directory for `AGENTS.md` (not `CLAUDE.md`) containing the
      expected Soul+AgentMD+Memory+Skills content.
- [ ] Armory → Memory: confirm the "Applies to" chips render and list the
      expected filename groups.

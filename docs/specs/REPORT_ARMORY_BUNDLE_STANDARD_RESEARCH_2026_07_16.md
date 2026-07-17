# Report: Is there a standard for Armory-style agent capability bundles? Research + proposal (2026-07-16)

**Status:** Research report + standard proposal — no implementation yet.
**Author:** Agent3
**Method:** Deep-research harness (5 parallel search angles → 18 sources
fetched → adversarial 3-vote verification per claim) + a code-level inventory
of the Armory's actual schemas on `main` @ `e6ec3c42`. The verification phase
was partially cut short by an org spend limit: claims marked **[verified]**
survived 3-0 adversarial votes; claims marked **[fetched]** were extracted
from primary sources with supporting quotes but did not complete adversarial
verification. No claim below is from model memory alone.
**Related:** `specs/SPEC_V1_MCP_SKILLS_PRIMITIVES_2026_06_30.md`,
`docs/specs/SPEC_PRESET_TO_BUNDLE_REFACTOR_2026_07_02.md`,
`docs/specs/archive/SPEC_RENAME_TRUST_CENTER_TO_ARMORY_2026_07_02.md`.

---

## 1. The question

AgentMux's Armory manages five primitives, composed per-agent:

| Armory primitive | Storage (today) | Materialized at launch as |
|---|---|---|
| **Bundles** (instructions + context files + refs) | `db_bundles` (instructions, `context_files` JSON, `mcp_servers` JSON, `skills` id-array, provider/model, `is_global`, `sort_order`) | CLAUDE.md sections via `format_global_brain_block` |
| **Accounts** (identity/credentials) | `db_accounts` with `secret_ref` tagged enum (`Env`/`SecretsManager`/`PlaintextDev`/`OAuthConfigDir`/`Keychain`) — plaintext never stored | env vars (`GITHUB_TOKEN`, `ANTHROPIC_API_KEY`, …) or config-dir injection (`CLAUDE_CONFIG_DIR`, `CODEX_HOME`) |
| **Skills** | `db_skills` (trigger, skill_type, content) | `.claude/commands/<trigger>.md` slash commands + CLAUDE.md index |
| **MCP servers** | `db_mcp_servers` (transport, config JSON) | standard `.mcp.json` `mcpServers` object |
| **Per-agent composition** | `db_agent_skills_ref`, `db_agent_mcp_ref`, `db_agent_identity_links` | — |

Everything is **SQLite-only**; there is no export/import/share path for any
Armory primitive (the only exporter, `exportagents`, covers agent definitions,
not bundles). The question: does a standard exist for storing/packaging/
distributing this kind of composed capability bundle — and if not, what's
closest, and what should AgentMux adopt?

## 2. Answer

**No standard — formal or de facto — bundles all four categories
(instructions/memory, MCP configs, skills, credential references) as of
mid-2026.** What exists instead is a *layered* landscape: strong per-category
standards for two of the four (skills; MCP server description), a
foundation-governed convention for a third (instructions), a **universal,
deliberate refusal to package the fourth (credentials)** across every format
examined, and two young multi-category composition efforts (Microsoft APM,
Appstrate AFPS) neither of which covers everything. The strategic conclusion
writes itself: **adopt the per-category standards where they exist, and
standardize only the composition layer + credential-reference schema
ourselves** — which happens to be exactly the two things the Armory already
does in proprietary form.

## 3. The landscape, per category

### 3a. Skills — a real, won standard exists: Agent Skills (SKILL.md)

- **[verified]** A skill is a directory with a required `SKILL.md` (YAML
  frontmatter + Markdown instructions) and optional `scripts/`, `references/`,
  `assets/` dirs. It standardizes the on-disk format — not an archive,
  registry, or distribution mechanism. (agentskills.io; github.com/agentskills/agentskills; anthropics/skills spec)
- **[verified]** Created by Anthropic, released as an open standard (announced
  2025-12-18), now governed in a vendor-neutral GitHub org
  (`agentskills/agentskills`, Apache-2.0/CC-BY-4.0); the spec in Anthropic's
  own repo is a stub redirecting to agentskills.io.
- **[verified]** Broad cross-vendor adoption (~45 listed clients incl. OpenAI
  Codex, Gemini CLI, GitHub Copilot, VS Code, Cursor, Goose, JetBrains,
  Mistral) — the de facto standard for this category.
- **[verified]** Frontmatter has exactly six fields (`name`, `description`
  required; `license`, `compatibility`, `metadata`, `allowed-tools` optional).
  **No fields for credentials, MCP config, or memory** — it covers category
  (c) only. Progressive disclosure (metadata ~100 tokens → body on activation
  → files on demand) is part of the spec.
- **Gap vs Armory:** `db_skills` are *not* SKILL.md — they generate
  slash-commands (`.claude/commands/<trigger>.md`). This is now swimming
  against a ~45-client standard.

### 3b. MCP server configs — standardized description + multiple distribution channels

- **[fetched]** `server.json` is the MCP project's standardized server
  description format (registry publishing, client discovery, package
  management), with a date-versioned hosted JSON Schema
  (`static.modelcontextprotocol.io/schemas/2025-12-11/server.schema.json`).
  It references packages in existing ecosystems — `registryType`: npm, pypi,
  cargo, nuget, **oci**, **mcpb** — rather than defining a new archive.
- **[fetched]** The official MCP Registry (backed by Anthropic, GitHub,
  PulseMCP, Microsoft) hosts metadata only — no code/binaries — and is still
  **in preview** with possible breaking changes. Trust model = namespace
  authentication (reverse-DNS names) only.
- **[fetched]** Credentials appear only as declarative placeholders
  (`isSecret`/`isRequired` env flags) — the format tells clients *what to
  collect*, never carries values.
- **[fetched]** MCPB (MCP Bundle, formerly `.dxt`) is Anthropic's zip-based
  one-click package for a *local* server + deps (manifest.json required),
  versioned 0.1, with `user_config` sensitive values stored host-side in the
  OS keychain. Docker's MCP Catalog distributes servers as signed OCI images
  with SBOM/provenance (~300+ servers), with OAuth handled at runtime by the
  toolkit — again, credentials never in the artifact.
- **Armory position:** `db_mcp_servers.config` already merges into standard
  `mcpServers` — this category is the closest to aligned. Adding
  `server.json` import (and its `isSecret` env declarations mapped onto
  Armory account references) is incremental.

### 3c. Instructions/memory — a convention, not a schema: AGENTS.md

- **[fetched]** AGENTS.md is stewarded by the Agentic AI Foundation under the
  Linux Foundation (originated with OpenAI, Amp, Google Jules, Cursor,
  Factory); 60k+ repos and 20+ native tools as of May 2026. It standardizes
  *location and precedence* (nearest-file-wins, nested scoping), **not
  structure** — plain Markdown, no schema, no frontmatter.
- **[fetched]** It explicitly excludes the other categories (no credentials
  ever; MCP is "a separate layer"; SKILL.md is "Anthropic's separate spec").
  Claude Code notably still reads CLAUDE.md, with `@AGENTS.md` referencing as
  the interop workaround; Codex caps the file at 32 KiB.
- **[fetched]** Letta's `.af` (Agent File) is the only found format that
  serializes memory + tools together (system prompt, editable memory blocks,
  tool code, model settings) — but it strips secrets to `null` on export, has
  MCP support only on its roadmap, and adoption is effectively single-vendor.
- **Armory position:** `db_bundles.instructions` + `context_files` are
  functionally "portable AGENTS.md fragments + attachments" — near-alignable.

### 3d. Credentials/identity — no standard exists, by universal design

Every format examined **refuses** to package secrets, converging on the same
pattern AgentMux already implements (`SecretRef` pointers):

- **[fetched]** MCP delegates identity entirely to OAuth 2.1 + IETF standards
  (RFC 8414/7591/9728/8707) for remote servers, and standardizes *nothing*
  for local/stdio credential provisioning ("use env vars"). Official guidance:
  never embed credentials in packaged code.
- **[fetched]** Dev Containers marks secrets explicitly out of scope for
  `devcontainer.json`; conforming implementations must supply their own
  secure mechanism (keychain, secrets file, key vault), with rotation
  decoupled from the artifact. (Status: implemented — spec issue #219.)
- **[fetched]** Claude Code plugins: `userConfig` prompts per-user at enable
  time; sensitive values go to the OS keychain, never the bundle. MCPB: same
  pattern. A2A Agent Cards: declare *auth schemes* only, recommend
  out-of-band dynamic credentials.
- **Armory position:** ahead of the field — `db_accounts.secret_ref` is
  already a typed reference system. What no one standardizes (and we can) is
  the **declaration** format: "this bundle requires a `github` account with
  scopes X" — resolved locally against the user's own accounts.

### 3e. Multi-category composition — the frontier, and it's young

- **[fetched]** **Microsoft APM** ("dependency manager for AI agents",
  ~3.3k stars, v0.25.0 released 2026-07-12): one `apm.yml` manifest + lockfile
  declaring instructions, skills, prompts, agents, hooks, plugins, and MCP
  servers — explicitly *composing* AGENTS.md + Agent Skills + MCP rather than
  inventing formats; installs MCP servers by registry reverse-DNS id and
  deploys cross-client (Copilot, Claude, Cursor, Codex, Gemini, Windsurf…).
  **Covers 3 of 4 categories — no credentials/identity story.**
- **[fetched]** **Claude Code plugins**: one directory bundling skills,
  subagents, hooks, MCP servers, LSP servers (manifest optional; only `name`
  required). Composes with standard `mcpServers`; **explicitly ignores
  CLAUDE.md at plugin root** (instructions must ship as skills); credentials
  via per-user `userConfig`→keychain. Its manifest deliberately tolerates
  foreign fields so one file can double as VS Code/npm/MCPB manifests — the
  clearest evidence that **no unified standard exists** and formats are being
  overlaid on shared files.
- **[fetched]** **Appstrate AFPS v2.0**: the only spec found that attempts
  credentials — one archive declaring agent + skills + MCP servers +
  integrations, with OAuth/OIDC discovery metadata and credential *delivery*
  schemas (env/http/files). Skills are a declared strict superset of Agent
  Skills. But: **no memory/instruction category, and zero adoption** ("None
  yet" under implementations) — a candidate spec, not a standard.
- **[fetched]** Distribution prior art converges on **OCI artifacts**: Dev
  Container Features (tgz layers, custom media types, full metadata as a
  manifest annotation for registry-side indexing, semver with immutable
  republish refusal, collection index files) is the most complete blueprint;
  Docker cagent (agent YAML as OCI artifact, instruction files inlined before
  push), KitOps ModelKits (CNCF-adjacent, explicitly lists "prompts, agent
  skill files, MCP server configurations" as packageable, model optional),
  and Docker's enterprise-catalog guidance (catalogs as immutable OCI
  artifacts; Cosign signing; per-team "profiles" as artifacts; prod pins
  v2.3 while QA runs v2.4) all reuse the container supply chain — signing,
  scanning, access control — instead of inventing registries.
- **[fetched]** LangChain Hub: prompts only, git-like immutable commit
  hashes + mutable `staging`/`production` tags — a useful versioning/promotion
  pattern, not a bundle standard. A2A (Linux Foundation, v1.0): discovery
  metadata (`/.well-known/agent-card.json`, RFC 8615), explicitly no registry
  API, no packaging.

## 4. Gap analysis: Armory vs the landscape

| Category | Standard exists? | Armory today | Distance |
|---|---|---|---|
| Skills | **Yes — Agent Skills (SKILL.md)**, ~45 clients | Proprietary slash-command rows | **Misaligned** — biggest single win available |
| MCP configs | **Yes — `server.json` + `mcpServers`** (registry in preview) | Emits standard `mcpServers` | Nearly aligned; add server.json import + `isSecret`→account-ref mapping |
| Instructions/memory | Convention only (AGENTS.md: location/precedence, no schema) | `instructions` + `context_files` in DB | Alignable by materializing as Markdown + files; no schema exists to adopt |
| Credentials | **No standard anywhere; universal reference-don't-bundle** | `SecretRef` typed pointers — ahead of field | Standardize the *requirement declaration*, keep resolution local |
| Composition | Young (APM 3/4 no-creds; AFPS 3/4 no-memory, no adoption; plugins no-instructions) | `db_bundles` + ref tables, DB-only | **This is the open space** — nothing owns it yet |
| Distribution | De facto substrate: OCI artifacts (devcontainer Features blueprint) | None (no export at all) | Green field; follow the Features pattern |

## 5. Proposal: the Armory Bundle Format ("ABF", working name)

Design principle, dictated by the research: **compose won standards; invent
only where nothing exists** (the composition manifest and the credential
requirement declaration) — the exact posture that made APM credible and that
AFPS's invent-everything approach failed to earn adoption with.

### 5.1 On-disk format (the unit of interchange)

```
my-bundle/
├── armory.json              # manifest (§5.2) — the only invented schema
├── instructions/
│   ├── AGENTS.md            # primary instruction file (AGENTS.md convention)
│   └── context/…            # context_files, referenced from AGENTS.md
├── skills/
│   └── <skill-name>/
│       └── SKILL.md         # Agent Skills spec, verbatim — no extensions
├── mcp/
│   └── <server-name>.server.json   # MCP server.json schema, verbatim
└── accounts/
    └── requirements.json    # credential REQUIREMENTS (§5.3) — never secrets
```

### 5.2 `armory.json` manifest

```jsonc
{
  "$schema": "https://agentmux.ai/schemas/armory-bundle/v0.1/bundle.schema.json",
  "name": "acme-backend-dev",            // reverse-DNS optional for registry use
  "version": "1.2.0",                    // semver; immutable once published
  "description": "Backend dev bundle: repo conventions, GH tooling, deploy skills",
  "provider": { "preferred": "claude", "model": "claude-sonnet-5" },  // hint, not constraint
  "components": {
    "instructions": ["instructions/AGENTS.md"],      // ordered (maps sort_order)
    "skills": ["skills/deploy-checklist"],
    "mcpServers": ["mcp/github.server.json"],
    "accounts": "accounts/requirements.json"
  },
  "compatibility": { "agentmux": ">=0.54" },
  "metadata": {}                          // free-form, foreign fields tolerated
}
```

Foreign-field tolerance is deliberate (the Claude-plugin lesson): a bundle
dir can simultaneously be a valid Claude Code plugin or carry npm metadata.

### 5.3 `accounts/requirements.json` — the piece nobody else standardizes

Declares *what identities the bundle needs*, resolved at import/launch
against the user's local `db_accounts`; secrets never serialize:

```jsonc
{
  "requirements": [
    {
      "id": "gh-main",
      "provider": "github",              // matches db_accounts.provider
      "kind": "api-key | oauth",         // matches db_accounts.kind
      "scopes": ["repo", "workflow"],    // advisory; shown at import
      "env": "GITHUB_TOKEN",             // where it lands (resolver already does this)
      "optional": false
    }
  ]
}
```

This unifies the three patterns found in the wild — MCPB/plugins'
"sensitive user_config prompt", `server.json`'s `isSecret` env declarations,
and devcontainers' "implementation-provided secure mechanism" — into one
declaration that maps 1:1 onto the existing `SecretRef` resolution.

### 5.4 Distribution (phase-gated, not required for v0.1)

Directory + zip first (interchange by file). Then OCI artifacts following the
Dev Container Features blueprint: custom media type
(`application/vnd.agentmux.bundle.v1+tar`), full `armory.json` embedded as a
manifest annotation for registry-side indexing without pulls, semver tags
with immutable-republish refusal, Cosign-compatible. This reuses the
container supply chain AgentMux already ships with (signing, scanning,
registry auth) — no new infrastructure invented.

## 6. Implementation steps for AgentMux

Ordered so every phase is independently shippable and none blocks the UI:

**Phase 0 — align skills with the Agent Skills standard.** Add SKILL.md
support to `db_skills` (either a `format` column distinguishing
`slash-command` | `agent-skill`, or migrate content to SKILL.md frontmatter+
body and *derive* the slash-command materialization from it). At launch,
materialize agent-skill-format entries as `.claude/skills/<name>/SKILL.md`
(native Claude Code consumption) instead of only `.claude/commands/`. This is
the single highest-leverage step: it makes every existing community skill
(~45-client ecosystem) importable into the Armory as-is.

**Phase 1 — exporter.** `armory export bundle <id> --dir/-o zip`: serialize a
`db_bundles` row + its referenced skills/MCP servers into the §5.1 layout —
instructions→`AGENTS.md`, `context_files`→`instructions/context/`, skills→
SKILL.md dirs, `mcp_servers`→`server.json` files, `db_agent_identity_links`-
implied needs→`requirements.json`. Pure read-side; zero schema risk. (Also
covers the "back up my Armory" ask implicitly.)

**Phase 2 — importer + validation.** JSON-Schema validation of `armory.json`,
Agent Skills reference validation for `skills/` (the `skills-ref` library
exists), `server.json` schema validation for `mcp/`. On import: create rows,
then run **account resolution** — match each requirement against
`db_accounts` by provider/kind, prompting to link or create (with `SecretRef`
choice) for misses. Never import secret material even if present in a
malicious bundle (reject files under `accounts/` other than
`requirements.json`).

**Phase 3 — Armory UI.** Export/Import buttons on the Bundles rail; an
import-review sheet showing exactly what will be created (instructions
preview, skills list, MCP servers with their `isSecret` env needs, account
requirements with resolution status) — the trust-review moment every format
in the wild delegates to "install from trusted sources"; we can do better
because the requirements are declared.

**Phase 4 — OCI distribution.** `armory push/pull` against any OCI registry
via the Features-blueprint packaging (§5.4). Ties into the existing
container-toolchain detection. Private registries work day one (Harbor/
Artifactory/GHCR); this is also the natural enterprise story (per-team
profiles as pinned artifacts — Docker's model).

**Phase 5 — registry + spec publication.** An `agentmux-cloud` bundle index
(metadata-only, MCP-Registry model: point at OCI refs, don't host) and
publishing the `armory.json` + `requirements.json` schemas publicly
(versioned URLs, as `server.json` does). If APM or the Agent Skills org later
grows a composition standard, the format's compose-don't-invent posture makes
converging cheap — worth engaging APM (its no-credentials gap is exactly our
contribution) before minting anything as "1.0".

## 7. Verification caveats

The spend-limit interruption means claims in §3b, §3c (AGENTS.md adoption
numbers, `.af` roadmap), §3d, and §3e carry **[fetched]** status: extracted
from the named primary sources with supporting quotes, but without the
3-vote adversarial pass that §3a's claims passed. Before any external-facing
publication of this report's numbers (60k repos, ~300 servers, APM release
dates, AFPS adoption), re-verify against the live sources. None of the
*directional* conclusions (no all-four standard; credentials universally
excluded; OCI as substrate; skills standard won) rest on a single unverified
number.

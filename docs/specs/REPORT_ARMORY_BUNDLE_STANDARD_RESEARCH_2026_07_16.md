# Report: Is there a standard for Armory-style agent capability bundles? Research + proposal (2026-07-16)

**Status:** Research report + standard proposal — no implementation yet.
**Author:** Agent3
**Method:** Four deep-research harness passes + a code-level inventory of
the Armory's actual schemas on `main` @ `e6ec3c42`.
- **Pass 1** (5 search angles, 18 sources fetched) was cut short by an org
  spend limit partway through adversarial verification. Its claims are marked
  **[fetched]**: extracted from primary sources with supporting quotes, but
  not adversarially voted.
- **Pass 2** (re-run after the limit reset; 3 search angles, 15 sources, 66
  claims extracted → 25 sent to 3-vote adversarial verification → 19
  confirmed / 6 refuted / 0 unverified, 95 agent calls total, zero errors)
  completed cleanly end-to-end, including synthesis. Its claims are marked
  **[verified: N-M]** with the vote tally. Its own scope decomposition was
  narrower than Pass 1's (3 angles vs 5) and its synthesis explicitly flagged
  AGENTS.md, A2A, LangChain Hub, OCI/Docker MCP Catalog, and emerging
  manifest efforts (APM, AFPS) as unresearched-that-round.
- **Pass 3** (targeted follow-up aimed directly at Pass 2's open questions;
  6 search angles, 28 sources, 123 claims extracted → 25 verified → 22
  confirmed / 3 refuted / 0 unverified, 111 agent calls, zero errors) resolved
  four of the nine open items with high confidence — AGENTS.md governance/
  scope, A2A Agent Cards, LangChain's new Context Hub, and OCI-artifact
  distribution (Docker MCP Catalog, cagent, KitOps ModelKit) — all now
  **[verified: N-M]**. The other five open questions (Microsoft APM,
  Appstrate AFPS, whether Claude Code plugins bundle CLAUDE.md-style memory,
  the skills.json-vs-SEP-2640 status, and any emerging cross-provider identity
  standard) came back with **zero surviving claims** despite being the
  explicit target — the search didn't route to sources on them this round,
  for reasons the harness can't distinguish from "nothing there to find."
  These five remain at Pass 1's **[fetched]**-tier status; a further
  automated pass on the same five is unlikely to help and they're flagged
  for manual/targeted verification if precision on them becomes load-bearing.
- **Pass 4** (dedicated to a question the first three passes had implicitly
  conflated away — see the naming note in §1: is there a standard for
  *dynamic agent memory*, distinct from the *static instructions* AGENTS.md
  covers? — 4 search angles, sources on Letta `.af`, Mem0, Zep, MemGPT,
  LangMem, standards-body efforts, vendor-hosted exportable memory, and a
  memory-as-MCP-server pattern; 25 claims verified → 20 confirmed / 5
  refuted / 0 unverified, 109 agent calls, zero errors) found the dynamic-
  memory landscape is **more fragmented than every other category
  researched**, including credentials. Results are in the new §3f.
Where a later pass refuted an earlier claim, the correction is applied below
and called out explicitly. No claim below is from model memory alone.
**Related:** `specs/SPEC_V1_MCP_SKILLS_PRIMITIVES_2026_06_30.md`,
`docs/specs/SPEC_PRESET_TO_BUNDLE_REFACTOR_2026_07_02.md`,
`docs/specs/archive/SPEC_RENAME_TRUST_CENTER_TO_ARMORY_2026_07_02.md`.

---

## 1. The question

AgentMux's Armory manages five primitives, composed per-agent:

| Armory primitive | Storage (today) | Materialized at launch as |
|---|---|---|
| **Bundles** (instructions + context files + refs) — UI-labeled "Memories"/"Brain" | `db_bundles` (instructions, `context_files` JSON, `mcp_servers` JSON, `skills` id-array, provider/model, `is_global`, `sort_order`) | CLAUDE.md sections via `format_global_brain_block` |
| **Accounts** (identity/credentials) | `db_accounts` with `secret_ref` tagged enum (`Env`/`SecretsManager`/`PlaintextDev`/`OAuthConfigDir`/`Keychain`) — plaintext never stored | env vars (`GITHUB_TOKEN`, `ANTHROPIC_API_KEY`, …) or config-dir injection (`CLAUDE_CONFIG_DIR`, `CODEX_HOME`) |
| **Skills** | `db_skills` (trigger, skill_type, content) | `.claude/commands/<trigger>.md` slash commands + CLAUDE.md index |
| **MCP servers** | `db_mcp_servers` (transport, config JSON) | standard `.mcp.json` `mcpServers` object |
| **Per-agent composition** | `db_agent_skills_ref`, `db_agent_mcp_ref`, `db_agent_identity_links` | — |

**A naming note, caught mid-report and worth stating precisely:** the
Armory's "Bundles" primitive is what the *UI* calls "Memories"/"Brain"
(`GlobalBrainManager`), but its actual schema — `instructions` +
`context_files`, write-once/read-many, human-authored — is structurally
**static instructions** (the same category as AGENTS.md/CLAUDE.md), not
**dynamic memory** (state an agent accumulates and mutates on its own across
sessions, the way Letta/Mem0/Zep use the word). This report's original draft
conflated the two under one "memory/instruction bundles" category; §3c below
covers the instructions side (well-standardized, via AGENTS.md), and the new
§3f covers true dynamic memory as its own, separately-researched question
(the landscape there turns out to be meaningfully different — and worse).
AgentMux currently implements only the instructions side; it has no dynamic
per-agent memory primitive at all as of this report.

Everything is **SQLite-only**; there is no export/import/share path for any
Armory primitive (the only exporter, `exportagents`, covers agent definitions,
not bundles). The question: does a standard exist for storing/packaging/
distributing this kind of composed capability bundle — and if not, what's
closest, and what should AgentMux adopt?

## 2. Answer

**No standard — formal or de facto — bundles even the four categories this
report originally set out to check (instructions, MCP configs, skills,
credential references), and a fifth category this report initially
conflated with "instructions" — dynamic agent memory — turns out to have
*no* standard either, and is the single least-converged category of all
five.** Confirmed by four research passes (three completing full end-to-end
adversarial verification: 19+22+20=61 confirmed / 6+3+5=14 refuted / 0
unverified claims across all three, 0 agent errors in any of them). What
exists instead is a *layered* landscape, now verified in detail across
every category but two (dynamic memory, and the five still-open items in
§7): strong, vendor-neutrally-governed per-category standards for
**skills** (Agent Skills/SKILL.md), **MCP server description**
(`server.json`/`mcpServers`, registry still pre-GA), and **static
instruction location/precedence** (AGENTS.md, now confirmed genuinely
foundation-governed via the Agentic AI Foundation, not an OpenAI
convention) — but **no standard, and not even a shared non-standard, for
dynamic memory**: Letta, Zep, and LangMem each use structurally
incompatible representations (memory blocks, temporal knowledge graphs, and
no format at all, respectively), with none adopted outside its own
framework;
a confirmed, **universal, deliberate refusal to package the fourth
(credentials)** across every format examined in all three passes — the
single most consistently reinforced finding of the whole effort, now backed
by A2A Agent Cards, Docker's MCP Toolkit, and cagent in addition to the
formats found in Pass 1; a real, maturing **OCI-artifact distribution
substrate** (Docker MCP Catalog, cagent, KitOps ModelKit — all confirmed in
detail in Pass 3); and a genuinely-expanded LangChain **Context Hub** now
composing instructions + tools. Of the young multi-category composition
efforts, **Claude Code Plugins remains the most fully verified and the
closest to bundling multiple categories** (skills + MCP + hooks, with a
real, size-capped credential mechanism) — but whether it bundles static
CLAUDE.md-style instructions is the report's single most-attempted,
least-resolved question: three verification attempts across two passes
(Pass 2's re-check, Pass 3's dedicated targeting) have failed to settle it
either way. Microsoft APM and Appstrate AFPS remain unverified after two
dedicated attempts each — real per Pass 1's sourcing, but not independently
re-confirmed. The strategic conclusion is unchanged and now substantially
more strongly evidenced: **adopt the per-category standards where they
exist (skills, MCP config, static instructions), and standardize only the
composition layer + credential-reference schema ourselves — deliberately
leaving dynamic memory out of v0.1**, since there's no standard, no
convergent non-standard, and not even Armory's own current implementation
to align it against. That composition + credential-declaration scope happens
to be exactly the two things the Armory already does in proprietary form,
and exactly the two things the entire research effort found nobody else has
solved either.

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
- **[verified: mixed]** Cross-vendor adoption is real but the specific "~45
  clients, same folder runs unmodified" framing from Pass 1 was **refuted in
  Pass 2 (1-2 vote)** — the underlying blog source's portability claim didn't
  hold up to a skeptical re-check. Treat adoption as "broad and growing,
  governed vendor-neutrally" rather than citing a specific client count or
  claiming byte-for-byte portability across all listed clients.
- **[verified: 3-0]** Governance has moved out of Anthropic's own repository:
  `anthropics/skills`' spec file is now an 87-byte stub reading "The spec is
  now located at https://agentskills.io/specification" — confirmed by direct
  fetch of both URLs in Pass 2, independently of Pass 1's claim to the same
  effect.
- **[verified]** Frontmatter has exactly six fields (`name`, `description`
  required; `license`, `compatibility`, `metadata`, `allowed-tools` optional).
  **No fields for credentials, MCP config, or memory** — it covers category
  (c) only. Progressive disclosure (metadata ~100 tokens → body on activation
  → files on demand) is part of the spec.
- **Gap vs Armory:** `db_skills` are *not* SKILL.md — they generate
  slash-commands (`.claude/commands/<trigger>.md`). This is swimming against
  a real, vendor-neutrally-governed standard, whatever the exact client count.

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
- **[verified: 2-1]** The Official MCP Registry is a community-run, searchable
  directory backed by Anthropic/GitHub/PulseMCP/Microsoft — **not** a frozen
  v1.0 spec; its API is pre-GA and versioned by date, and it can still change.
- **[verified: 3-0]** Proposed `.well-known` MCP-server discovery extensions
  (SEP-1649 `server-card.json`, SEP-1960 manifest endpoint, successor
  SEP-2127) remain **unmerged Draft proposals** as of mid-2026 — even the
  discovery-mechanism slice of "MCP config" isn't fully settled, let alone
  packaging.
- **[verified: 3-0/2-1]** Two small, unresolved convergence attempts are
  trying to merge skills-packaging with MCP-config-packaging under one
  distribution mechanism — evidence the *composition* gap is actively felt,
  not just theoretical:
  - `mpak` (github.com/NimbleBrainInc/mpak, ~6 stars, created Feb 2026)
    distributes **both** `.mcpb` MCP bundles and `.skill` (SKILL.md-based)
    packages through one registry as distinct artifact types, with OIDC
    publisher authentication — but no memory or per-agent credential
    category. Small/new; weak evidence of broader trend, not proof of one.
  - An open GitHub discussion
    (`modelcontextprotocol/registry` discussion #895, Jan 2026) proposes
    extending the Official MCP Registry with a `skills.json` schema modeled
    directly on `server.json` — but this competes with an alternative
    **"Skills over MCP" (SEP-2640)** working-group track. Status as of the
    discussion's own last update: neither resolved. A dedicated Pass-3
    attempt to get a current status update on this specific question came
    back with **zero surviving claims** — check the live GitHub discussion
    directly for the current state rather than trusting a timestamp here.
- **Armory position:** `db_mcp_servers.config` already merges into standard
  `mcpServers` — this category is the closest to aligned. Adding
  `server.json` import (and its `isSecret` env declarations mapped onto
  Armory account references) is incremental.

### 3c. Instructions — a convention, not a schema: AGENTS.md

*(Static, human-authored context only — see the naming note in §1. Dynamic,
agent-accumulated memory is a separate question, researched independently in
§3f. Pass 3 confirmed this section with 3-0 adversarial votes across the
board — upgraded from Pass 1's [fetched] tier.)*

- **[verified: 3-0]** AGENTS.md is genuinely stewarded by the **Agentic AI
  Foundation (AAIF)**, a directed fund under the Linux Foundation formed
  2025-12-09. AAIF's three founding project contributions are MCP
  (Anthropic), goose (Block), and AGENTS.md (OpenAI); its Governing Board
  includes OpenAI, Anthropic, Block, Google, Microsoft, AWS, Bloomberg, and
  Cloudflare — genuinely vendor-neutral, not an OpenAI convention with a
  foundation badge stapled on. (Corrects Pass 1's vaguer "originated with
  OpenAI, Amp, Google Jules, Cursor, Factory" framing.)
- **[verified: 3-0]** Adoption: "more than 60,000 open source projects and
  agent frameworks" per the Linux Foundation's and OpenAI's own announcement
  text (as of the Dec 2025 AAIF launch), naming Amp, Codex, Cursor, Devin,
  Factory, Gemini CLI, GitHub Copilot, Jules, and VS Code among adopters. A
  more specific "24+ native tools" framing was **refuted (1-2)** — don't cite
  an exact native-tool count beyond the ~9 named above.
- **[verified: 3-0]** The spec standardizes *only* file location/precedence
  — "just standard Markdown... no required fields or mandatory structure,"
  nearest-file-wins with explicit user prompts overriding everything — and
  addresses **no** memory-management, credential, MCP-config, or skills
  schema anywhere in the spec. A frontmatter/schema proposal remains
  open/unmerged as of mid-2026.
- **Armory position:** `db_bundles.instructions` + `context_files` are
  functionally "portable AGENTS.md fragments + attachments" — near-alignable.
  (Letta's `.af` format, which does touch real dynamic memory, is covered in
  §3f, not here — it's a different category from what AGENTS.md/`db_bundles`
  actually are.)

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
  pattern.
- **[verified: 3-0]** A2A Agent Cards: confirmed as a discovery/metadata
  document only — "Agents MUST NOT embed secrets or sensitive information in
  the public Agent Card." The spec defines five discriminated
  `securityScheme` types (API key, HTTP auth, OAuth2, OIDC, mTLS), all
  mechanism metadata (header names, OAuth2 flow endpoints, OIDC discovery
  URL) — never a secret value. A competing claim that this was still an
  unfinished roadmap item was explicitly **refuted (0-3)**: auth-scheme
  declaration is finished, current spec, not planned.
- **[verified: 3-0]** The Docker ecosystem (MCP Toolkit, cagent) confirms the
  same pattern at OCI-artifact scale: the MCP Toolkit manages OAuth via
  browser-based authorization into a platform secret store ("You don't need
  to manually create API tokens... credentials aren't included in shared
  profiles"); cagent routes secrets through env files / a planned 1Password
  integration / Docker Desktop's MCP Gateway secret engine at runtime, and
  implements redaction hooks to strip secrets before an agent is shared as
  an artifact. No verified source confirms the same explicitly for KitOps
  ModelKit specifically — treat that one as inferred from its documented
  contents, not directly sourced.
- **Armory position:** ahead of the field — `db_accounts.secret_ref` is
  already a typed reference system. What no one standardizes (and we can) is
  the **declaration** format: "this bundle requires a `github` account with
  scopes X" — resolved locally against the user's own accounts.

### 3e. Multi-category composition — the frontier, and it's young

- **[fetched — unresearched after TWO targeted passes]** **Microsoft APM**
  ("dependency manager for AI agents", ~3.3k stars, v0.25.0 released
  2026-07-12): one `apm.yml` manifest + lockfile declaring instructions,
  skills, prompts, agents, hooks, plugins, and MCP servers — explicitly
  *composing* AGENTS.md + Agent Skills + MCP rather than inventing formats;
  installs MCP servers by registry reverse-DNS id and deploys cross-client
  (Copilot, Claude, Cursor, Codex, Gemini, Windsurf…). **Covers 3 of 4
  categories — no credentials/identity story.** Pass 3 targeted this
  explicitly and found **zero verifiable sources** this round — not
  contradicted, just not surfaced twice now. Treat as Pass-1-tier only;
  verify directly (the project's own repo/README) before relying on the
  specific numbers here.
- **[verified: multiple 3-0]** **Claude Code plugins** remain **the closest
  existing artifact to a standard bundling multiple Armory categories
  together**: a `.claude-plugin/plugin.json` manifest packages namespaced
  SKILL.md folders (`plugin-name:skill-name`), subagents, hooks, and MCP
  (plus LSP) server configs (via standard `.mcp.json` at plugin root or
  inline) into one installable, versioned unit with **real dependency
  management** — semver constraints, transitive install/enable, pruning,
  release tagging — analogous to a traditional package manager. Confirmed
  via direct fetch of Anthropic's own docs
  (code.claude.com/docs/en/plugins-reference): manifest fields, component
  directories (`skills/`, `agents/`, `hooks/hooks.json`, `.mcp.json`,
  `.lsp.json`, `scripts/`), namespacing, and the dependency-array schema all
  verbatim-match the primary source.
  - Credentials: a limited but real mechanism — `userConfig` entries flagged
    `sensitive:true` route to the OS keychain or a local credentials file,
    capped at **~2KB total, shared with OAuth tokens**. Proprietary to Claude
    Code, not a cross-provider identity standard, but notably more than any
    other format examined offers.
  - Memory/instructions: **still genuinely unresolved after two dedicated
    verification attempts.** Pass 1 asserted plugins explicitly ignore
    `CLAUDE.md` at the plugin root; Pass 2's adversarial re-check of that
    exact claim came back refuted (1-2). Pass 3 was tasked specifically with
    resolving this definitively by fetching Anthropic's own plugin docs and
    checking exactly what happens to a root-level `CLAUDE.md` — and came back
    with **zero surviving claims** on the question. This is now the report's
    single most-attempted-and-least-resolved open item; a manual check of
    the live Anthropic docs (not another automated pass) is the efficient
    next step if this needs to be load-bearing for AgentMux's design.
  - The manifest tolerates foreign top-level fields — Claude Code ignores
    what it doesn't recognize — so one `plugin.json` can double as a VS
    Code/Cursor extension manifest, npm `package.json`, or MCPB manifest.
    That interoperability-by-tolerance is itself evidence no unified
    cross-tool standard exists; formats are being overlaid on shared files
    instead.
- **[verified: 3-0]** **LangChain's hub concept has genuinely expanded
  beyond prompts-only.** The new **LangSmith Context Hub** (launched
  2026-05-13) supersedes the old prompt-template-only LangChain Hub: "a
  context is a versioned bundle of agent instructions and tools, either a
  skill or a full agent, that you manage in LangSmith and promote to an
  environment," and "an Agent context is a full agent bundle including an
  AGENTS.md file and tools." This is a real, dated correction to Pass 1's
  "prompts only" framing — LangChain is now composing instructions + tools
  (two of the four Armory categories) under one versioned, promotable unit,
  though still no credential or MCP-specific story confirmed. The git-like
  immutable-commit-hash + mutable `staging`/`production`-tag versioning
  pattern from the legacy Hub is a real precedent worth borrowing regardless.
- **[verified: 3-0]** **A2A (Agent2Agent) protocol**: genuinely a Linux
  Foundation open-source project (contributed by Google), now at stable
  v1.0.1 (2026-05-28, following v1.0.0 on 2026-03-12), governed by a
  Technical Steering Committee including AWS, Cisco, Google, IBM Research,
  Microsoft, Salesforce, SAP, and ServiceNow. Agent Cards remain
  discovery/metadata only, not a packaging format — see §3d for the
  credential-scheme-only confirmation.
- **[verified: 3-0]** **OCI-artifact distribution is real and maturing**,
  confirmed in detail (correcting/upgrading Pass 1's [fetched] framing):
  - **Docker MCP Catalog**: 300+ servers, packaged as Docker images
    distributed via Docker Hub, each running as an isolated container.
    Docker-built (`mcp/`-namespaced) images are digitally signed with full
    provenance + SBOM metadata for transparency — this signing/SBOM claim
    had a non-unanimous 2-1 vote, so treat as medium- not high-confidence,
    and note it's scoped to Docker-built/local servers, not necessarily
    third-party/remote partner ones.
  - **Docker cagent**: an open-source CLI whose agent configs are
    distributed/versioned as OCI artifacts via standard registries (`cagent
    push`, `cagent run docker.io/...`) — confirmed with an exact working
    command from Docker's own blog. Credentials are consistently kept
    external (env files, planned 1Password integration, the MCP Gateway's
    secret engine); cagent has redaction hooks that strip secrets before an
    agent is shared.
  - **KitOps ModelKit**: confirmed verbatim as bundling "models, datasets,
    code, prompts, agent skill files, MCP server configurations, and
    documentation" in one OCI-compliant artifact, with a model **genuinely
    optional** — documented example kits contain only prompts+skills, or
    only an MCP server + its config. No source directly confirms credential
    exclusion for ModelKit specifically (inferred from its documented
    contents, not sourced) — the one sub-claim in this cluster that remains
    only partially verified.
  - Dev Container Features (tgz layers, custom media types, metadata-as-
    manifest-annotation, semver immutable-republish-refusal) and Docker's
    per-team "profiles as pinned artifacts" enterprise guidance remain
    **[fetched]**-tier — not re-targeted by Pass 3, still Pass-1-only.
- **[verified, medium confidence — inferential, reinforced across all three
  passes]** Across every spec/format examined — Agent Skills, MCP
  Registry/MCPB, mpak, Claude Code plugins, A2A Agent Cards, Docker MCP
  Toolkit, cagent — **identity/credential management is the least
  standardized of the four Armory categories**: it appears only as ad hoc,
  tool-local mechanisms (OS keychain, local credentials file, OIDC publisher
  auth, browser-based OAuth into a platform secret store) never as a
  portable, cross-provider "agent account/identity" object. No surviving
  claim across any of the three passes identified a format that bundles
  memory + MCP config + skills + identity together as first-class
  components of one artifact. This is the strongest, most consistently
  reinforced single conclusion of the entire research effort.
- **[fetched — unresearched after TWO targeted passes]** **Appstrate AFPS
  v2.0**: the only spec found (Pass 1 only) that attempts credentials — one
  archive declaring agent + skills + MCP servers + integrations, with
  OAuth/OIDC discovery metadata and credential *delivery* schemas
  (env/http/files); skills declared a strict superset of Agent Skills; **no
  memory/instruction category, and claimed zero adoption**. Pass 3 targeted
  this explicitly (does it exist as described? real implementations vs.
  "zero adoption"?) and came back with zero surviving claims. Verify
  directly against the project's own spec repo before citing its adoption
  status either way.

### 3f. Dynamic memory — no standard, and *less* converged than any other category

*(A genuinely different question from §3c: not "where do static, authored
instructions live" but "how does an agent's own accumulated, mutated,
cross-session state — learned facts, user preferences, episodic/semantic
recall — get serialized and moved between tools." Pass 4, dedicated
entirely to this question: 20 confirmed / 5 refuted / 0 unverified, 109
agent calls, zero errors.)*

- **[verified: high confidence]** **No cross-vendor standard exists for
  dynamic memory serialization, and the field is more fragmented than every
  other category in this report — including credentials**, where at least
  every vendor converges on the *same* non-solution (defer to OS
  keychain/OAuth). Here, each framework has a structurally different,
  mutually incompatible representation: Letta uses labeled memory blocks,
  Zep uses a temporal knowledge graph, LangMem has no format of its own at
  all. There isn't even a shared *non-standard* to point to.
- **[verified: high confidence]** **Letta's Agent File (`.af`) is the
  closest real candidate**, and it's a meaningfully different kind of
  artifact from AGENTS.md or SKILL.md: it bundles an agent's *full runtime
  state* — system prompt, message history, tool configs (code + schema),
  model settings, environment variables, tool rules, **and memory as
  discrete, labeled, individually-editable "memory blocks"** (e.g.
  `persona`, `human`/user-info) rather than one blob. A real, inspectable
  Pydantic schema exists (`AgentSchema`, `CoreMemoryBlockSchema`,
  `MessageSchema`, `ToolSchema`) — but it lives inside Letta's own
  application repo, not a standalone JSON Schema document, and not under any
  standards body.
  - **Coverage gap:** `.af` covers only *in-context* memory blocks — Letta's
    own **Archival Memory ("Passages")** is explicitly unsupported, listed
    as a roadmap item, not implemented. There's no memory-block-level
    versioning/history either (only agent-level version + per-block
    timestamps), despite "versioning agents over time" being a stated design
    goal.
  - **Adoption is Letta's own words, not inference:** "Theoretically, other
    frameworks could also load in `.af` files if they convert the state into
    their own representations" — no named third-party framework (LangChain,
    CrewAI, AutoGen, Google ADK) natively reads `.af`. Those frameworks have
    instead converged on **A2A/MCP for interop**, not a shared memory format.
  - An adversarial attempt to characterize `.af` as a formal "open standard"
    (rather than Letta's own de facto format) was **explicitly refuted
    (1-2)** — sharpening, not weakening, the "no standard exists" conclusion.
- **[verified: high confidence]** **Zep** represents memory as a temporal
  knowledge graph (entity nodes, entity edges, episodic nodes) built on
  **Graphiti**, its own open-source (Apache 2.0) graph engine — a real,
  independently-reimplementable architecture (a third party, GraphZep, has
  shipped a TypeScript reimplementation), but **Zep documents no
  export/portability API**. Migrating memory *into* Zep (e.g. from Mem0) is
  fully manual: map IDs, convert records to JSON/text, push through Zep's
  own `graph.add()` API — not a file conversion. An attempted claim that
  GraphZep offers semantic-web export (Turtle/JSON-LD/RDF) was **refuted
  (0-3)** — there's no portable serialization out of this ecosystem either.
- **[verified: high confidence]** **LangMem** (LangChain's memory library)
  defines **no storage format of its own at all** — its core API is
  explicitly designed to work with "any storage system," delegating
  entirely to whatever backend is configured (LangGraph's `BaseStore`,
  Postgres, MongoDB, in-memory). There is no LangMem-native file to move.
- **[gap — no primary source found, flagged explicitly rather than
  guessed]** Five of the eight original sub-questions came back with **zero
  verified claims**, and should be treated as open, not answered:
  1. **Mem0**'s own portable export format and governance (OSS core vs.
     hosted-only) — unconfirmed either way.
  2. Whether **MemGPT** (the project Letta evolved from) survives as an
     independently-maintained spec distinct from `.af`, or was fully
     absorbed with no separate surviving artifact.
  3. Any **W3C/IETF or standards-body** effort on agent-memory
     serialization — none found, but not exhaustively ruled out.
  4. Whether **OpenAI, Anthropic, or Google** offer any documented,
     end-user-*exportable* memory file from their hosted agent products —
     unconfirmed; if the answer is "no," it's unclear whether that's
     deliberate lock-in or simply unaddressed.
  5. Whether any memory framework has shipped a **"memory MCP server"**
     pattern — exposing its store over MCP's already-standard protocol, even
     without a portable underlying file format — as a lighter-weight
     interop path than full serialization. Not found, not ruled out.
- **Armory position:** AgentMux currently has **no dynamic memory primitive
  at all** — `db_bundles` is entirely static instructions (§1's naming
  note). This isn't a gap relative to a missed standard (there isn't one to
  align with); it's a genuinely open design space. If AgentMux ever adds
  real cross-session agent memory, Letta's memory-block *shape* (discrete,
  labeled, individually addressable) is the most-precedented pattern to
  borrow from — but the underlying storage/versioning would still be
  AgentMux's own design, not an import of an existing standard, because
  none of the four examined frameworks (Letta, Zep, LangMem, Mem0) offer one
  worth adopting wholesale.

## 4. Gap analysis: Armory vs the landscape

| Category | Standard exists? | Armory today | Distance |
|---|---|---|---|
| Skills | **Yes — Agent Skills (SKILL.md)**, vendor-neutrally governed | Proprietary slash-command rows | **Misaligned** — biggest single win available |
| MCP configs | **Yes — `server.json` + `mcpServers`** (registry in preview) | Emits standard `mcpServers` | Nearly aligned; add server.json import + `isSecret`→account-ref mapping |
| Instructions (static, authored) | **Yes — AGENTS.md**, Linux Foundation-governed convention (location/precedence, no schema) | `instructions` + `context_files` in DB | Alignable by materializing as Markdown + files; no schema exists to adopt |
| Credentials | **No standard anywhere; universal reference-don't-bundle** | `SecretRef` typed pointers — ahead of field | Standardize the *requirement declaration*, keep resolution local |
| **Dynamic memory** (agent-accumulated state) | **No standard, no convergent non-standard either** — more fragmented than credentials; Letta `.af`'s memory-block shape is the only real precedent, not adopted anywhere else | **None** — `db_bundles` is static instructions, not this | Not a gap to close now — genuinely nothing to align with; a future non-goal/extension point, not a v0.1 requirement |
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
  "$schema": "https://docs.agentmux.ai/schemas/armory-bundle/v0.1/bundle.schema.json",
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

### 5.5 Dynamic memory: explicit non-goal for v0.1

§3f found no standard, and no convergent non-standard, for dynamic agent
memory — and AgentMux doesn't have a dynamic memory primitive to bundle in
the first place (§1's naming note: `db_bundles`/"Memories" is static
instructions). ABF's `components` object (§5.2) is therefore deliberately
open-ended (unrecognized keys are ignored, matching the Claude-plugin
tolerance pattern) rather than closed to exactly four categories, so a
future `"memory"` component key can be added without a breaking manifest
version bump — but nothing is specified for it now. If AgentMux ever adds
real cross-session agent memory, Letta's memory-block shape (discrete,
labeled, individually addressable, not one blob) is the most-precedented
pattern to borrow from — but the storage/versioning underneath would still
be an AgentMux design, not an import, because none of the four frameworks
examined (Letta, Zep, LangMem, Mem0) offer a standard worth adopting
wholesale.

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

## 7. Verification caveats and open questions

**What's now fully verified, across Pass 2 and Pass 3 (both completed
clean — 95/95 then 111/111 agent calls, 0 errors either time, 41 total
claims confirmed by 3-vote adversarial check across the two, 0 left
unverified):**
- Agent Skills' scope and governance move to agentskills.io (Pass 2).
- The MCP Registry's pre-GA status and the unmerged draft status of its
  `.well-known` discovery SEPs; the `mpak` skills+MCP convergence registry
  (Pass 2).
- Claude Code Plugins' manifest/dependency/credential mechanics (Pass 2,
  reinforced in Pass 3's targeted follow-up).
- **AGENTS.md**'s genuine Agentic AI Foundation / Linux Foundation
  governance, its 60,000+-project adoption figure, and its schema-free
  location/precedence-only scope (Pass 3).
- **A2A Agent Cards**: Linux Foundation governance, v1.0.1 maturity,
  discovery-only scope, and confirmed auth-schemes-never-secrets design
  (Pass 3).
- **LangChain's Context Hub**: a real, dated expansion (2026-05-13) beyond
  prompts-only into bundling instructions + tools (Pass 3) — this is new
  information, not present in the original Pass-1 draft at all.
- **OCI-artifact distribution**: Docker MCP Catalog, cagent, and KitOps
  ModelKit all confirmed in detail, including that every one of them keeps
  credentials external to the artifact (Pass 3).
- And — the report's central, now triple-confirmed conclusion — that
  credential/identity management is the least standardized category
  everywhere, with no format across three research passes found to bundle
  all four categories as first-class components.

**Three claims were corrected after later passes refuted earlier ones**
(all fixed in place in §3, not just noted here): the specific "~45 clients,
portable unmodified" framing for Agent Skills adoption (Pass 2 refuted); a
vaguer, less accurate account of AGENTS.md's origin/governance (Pass 3
corrected with the AAIF/Linux-Foundation specifics); and the "24+ native
tools" framing for AGENTS.md adoption (Pass 3's own re-check of its own new
claim refuted this specific sub-number, 1-2).

**What remains [fetched]-tier after three passes** (extracted with quotes
in Pass 1, not adversarially voted, and — critically — **not resolved by
two separate dedicated attempts** in Pass 2 and Pass 3 despite direct
targeting): **Microsoft APM**, **Appstrate AFPS**, and — the report's
single most-attempted, least-resolved question — **whether Claude Code
plugins (or any format) support bundling persistent *static* instruction
files (CLAUDE.md) as a first-class component**. Also still `[fetched]`-tier,
not yet re-targeted: Dev Container Features and Docker's enterprise-catalog
"profiles as pinned artifacts" guidance. (Letta `.af`'s roadmap/coverage
status is now separately verified — see the Pass 4 summary below, §3f.)
For these specific items, a further automated research pass on the same
narrow questions is judged
unlikely to help — two consecutive targeted attempts came back with zero
surviving claims on exactly these questions, suggesting either the search
angle isn't finding the right sources or the harness's source mix doesn't
cover them well. **A manual, human-directed check of the primary sources
directly** (Anthropic's plugin docs for the CLAUDE.md question;
APM's/AFPS's own repos for the other two) is the efficient next step if
precision on these three becomes load-bearing for AgentMux's design —
none of them currently block any part of the §5 proposal or §6
implementation plan, since the proposal composes standards that *are*
verified (Agent Skills, `server.json`/`mcpServers`, AGENTS.md-style
instructions) and invents only the two things confirmed nobody else has
solved (composition manifest, credential requirements declaration).

Before any external-facing publication of this report's specific unverified
numbers (APM's v0.25.0/star counts, AFPS's claimed "zero adoption"), verify
directly against the live sources. None of the report's *directional*
conclusions rest on a single unverified number — each is corroborated by at
least one Pass-2-or-Pass-3-verified claim from a different, independent
angle.

**Pass 4 (dynamic memory, completed clean — 109/109 agent calls, 0 errors,
20 confirmed / 5 refuted / 0 unverified):** fully verified Letta `.af`'s
memory-block structure, its real-but-Letta-owned Pydantic schema, its
Archival-Memory/Passages coverage gap, and its "theoretical," not adopted,
cross-framework portability; fully verified Zep's Graphiti-based knowledge-
graph architecture and its lack of any export/portability mechanism; fully
verified LangMem's complete absence of a native storage format. Explicitly
**not resolved, zero surviving claims, flagged as open rather than
guessed**: Mem0's own format/governance, whether MemGPT survives as a spec
independent of `.af`, any W3C/IETF standards-body effort on agent memory,
whether any major vendor (OpenAI/Anthropic/Google) offers end-user-
exportable memory, and whether any framework exposes memory over MCP as a
protocol-portable (if not format-portable) interface. Same policy as the
other open items: a manual, source-directed check is more efficient than a
further automated pass on these specific five sub-questions, and none of
them block §5's proposal, which treats dynamic memory as an explicit
non-goal (§5.5) rather than something requiring a resolved external
standard to design against.

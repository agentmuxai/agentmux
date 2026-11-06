# REPORT: Prior art for reusable computer-use skills, and what to call a "pack"

**Date:** 2026-11-05
**Author:** Manoz@Area54, for task #1631 (operator's question to AgentY, 2026-11-05)
**Status:** analysis — research only, no code. It adds to `REPORT_COMPUTER_USE_STATE_AND_APP_SKILL_PACKS_2026_11_05.md` (#1555) and does not repeat it. Sources were read on 2026-11-05. Where a page was read through a summariser, or a date or file was not seen, it says so.

## Decisions for the operator

| # | Decision | My recommendation |
|---|---|---|
| N1 | Name of the unit | **"Computer-use skill"** in prose and the docs; **"App skill"** where space is tight (a card badge). Retire "pack" and never use "plugin" (§4). |
| N2 | Format | Keep it a standard Agent Skills folder, with AgentMux's manifest, tests and records as extra files in it (§4). No change to what is built. |
| N3 | Tab | One list: computer-use skills show in the **Skills** tab with an "App" filter, not a separate Packs tab (§4). This is the only change that touches the approved UI, so it is your call. |

## 1. Who ships reusable per-app computer-use knowledge

Short answer: **almost nobody, and nobody ships the whole thing.** The units that exist are one of three kinds.

| Project | One unit holds | Shared / versioned | State |
|---|---|---|---|
| [CUA-Skill](https://arxiv.org/abs/2601.21123) (Microsoft, Jan 2026) | App, intent, typed argument slots, and a parameterised execution graph of GUI actions or script actions, guarded by UI checks. About 450 skills over 16 Windows apps (the paper gives 452 and 478 in different places) | Retrieval by embedding; no registry. I found no code release in the paper text I read (the part after about 39k characters was summarised) | Research; [project page](https://microsoft.github.io/cua_skill/) not read |
| [UFO²](https://github.com/microsoft/UFO) AppAgents | Per-app agent with help-doc, Bing, past-run and demonstration sources in a local vector store | Local store only; no packaging or sharing described | MIT, labelled LTS; README news stops at 2025-11 |
| [OpenAdapt](https://github.com/OpenAdaptAI/OpenAdapt) | A compiled workflow **bundle** recorded from a demonstration or a run, replayed with no model call; it checks before each save and reads back after | A directory; `emit-skill` writes an Agent Skill for it; admissions name "that exact version" (signed lifecycle record, not fetched). No public registry described | Active (commit 2026-11-06; its README is out of date against the release title) |
| [Skyvern](https://github.com/Skyvern-AI/skyvern) | A workflow of blocks (browser task, extract, validate, loop, code) | In Cloud or the local server; no template gallery or versioning found | Active; web only |
| [browser-use](https://github.com/browser-use/browser-use) | A saved run (`save_history`) replayed with new variables | Local JSON, "keep private"; no registry | Active; its `skills/` teach the product, not a site |
| [Cua](https://github.com/trycua/cua), [Agent S](https://github.com/simular-ai/Agent-S) | Generic skills (`gui-automation`); trajectories for training | Skill version is probably in `metadata.version` (a commit title; skill not opened) | Active; no per-app unit found |
| [Orca](https://github.com/stablyai/orca) `orca-computer-use`, Hermes `computer-use` | A **generic** skill: accessibility tree, clicks, menus, "prefer a programmatic path first" | Orca's real guide ships inside its executable, so I could not read it. Hermes page returned 404 | Generic, not per-app |
| [glincker/ableton-skills](https://github.com/glincker/ableton-skills) | 12 producer-knowledge `SKILL.md` files plus 4 commands, on top of an Ableton MCP server it does not control itself | `git clone` into `~/.claude/skills`; MIT; 45 stars, 33 commits; commit dates not seen; README roadmap items unchecked | Community, young |

What the units hold falls into three kinds: **know-how text** (Orca, glincker), **a replayable program** (CUA-Skill, OpenAdapt, Skyvern, browser-use), and **retrieved documents and past runs** (UFO²). A pack is the first kind with a ladder to the other two. None of the above carries a per-app-version range, a test, or a verification record. That part of the pack is new, not borrowed.

## 2. Is it all just skills? Where does "a skill plus more" go

Yes for the text, and the industry has settled on **plugin** for the "plus more".

- **Agent Skills** ([spec](https://agentskills.io/specification)): `SKILL.md` with `name` (64 characters, matches the folder), `description` (1024), optional `license`, `compatibility` (500 characters), `metadata` (string-to-string map) and experimental `allowed-tools`. There is **no `version` field**; the spec's own example puts one in `metadata`. "A skill directory may contain any files and directories beyond the required `SKILL.md`" (`scripts/`, `references/`, `assets/` are conventions). Anthropic released it on 2025-12-18; many tools read it (the site's client list did not load, so I cannot name them from it).
- **Claude Code plugins** ([reference](https://code.claude.com/docs/en/plugins-reference)): a directory with `.claude-plugin/plugin.json` (only `name` is required; `version`, `dependencies` with semver ranges on **other plugins**, `userConfig`) and skills, commands, agents, hooks, MCP servers. Distributed through a git **marketplace**. Updates key on `version` or the commit SHA. It has no field for the version of the app a plugin drives. It does have `claude plugin eval`, which writes a local result file that names the Claude Code version that ran it ([evals](https://code.claude.com/docs/en/plugin-evals), summariser-truncated); the result is not part of the manifest and nothing says it is checked at install.
- **MCP Bundles** ([`.mcpb`](https://github.com/modelcontextprotocol/mcpb)): a zip of a local MCP server plus `manifest.json` (`version`, `compatibility` with a semver range per client and platform, `user_config`, `_meta`). It packages the **server**, not the know-how. No test or verification field. Claude Code can load one from inside a plugin.
- **Closest precedent for our pack:** [Adobe's `adobe/skills`](https://github.com/adobe/skills) (Apache-2.0, 1,247 commits). One monorepo; `plugins/<product>/.claude-plugin/plugin.json` plus `skills/`; a per-plugin and per-skill semver (`metadata.version`), per-package tags and changelog by `semantic-release`, a `*.skill` file attached to each release; installable in Claude Code, Copilot CLI and with `npx skills add`. [Figma's `mcp-server-guide`](https://github.com/figma/mcp-server-guide) is the same shape (skills for the Figma MCP server, packaged for Claude, Cursor and Gemini). Both are "skills + MCP pointer, versioned like software". Neither has app-version ranges, tests or verification records.

I did not find an OpenAI equivalent; I did not search for one beyond Codex reading `SKILL.md`.

## 3. Do app vendors publish good skills for driving their own app?

Vendors publish skills, but **none that I found drives their app's UI**. They teach an agent to use the vendor's API or MCP server.

- **Adobe:** about 16 plugins (Analytics, CJA, Workfront, AEM, App Builder, Commerce, Creative Cloud/Firefly). Well structured (the sample `aa-kpi-pulse` is 302 lines in phases, with a template and guardrails). They call hosted MCP servers and CLIs. The Creative Cloud plugin is "for Claude desktop app only" and ships a `.mcp.json`; I did not read how it runs. Nothing for Photoshop or Premiere UI work that I saw.
- **Figma:** 16+ skill folders, all for the Figma MCP server's `use_figma` (Plugin API JavaScript). `figma-use` is 383 lines with 21 numbered rules and a `references/` folder; it is good, and it is API work. Figma's blog says skills in its own agent are "plain English" and publishable to a team; versioning is not described.
- **Anthropic's `docx`, `xlsx`, `pptx`, `pdf` skills** ([repo](https://github.com/anthropics/skills)): they edit **files** with `docx-js`, raw OOXML and `openpyxl`, then render through LibreOffice. They do not touch the Word or Excel window. They are source-available, not open source ("Proprietary. LICENSE.txt"), and I did not read the licence text. The `xlsx` skill warns that LibreOffice cannot evaluate XLOOKUP, FILTER and SORT. That is the **file route**, which our Word and Excel packs already put on the ladder next to COM.
- **Microsoft:** [`microsoft/skills`](https://github.com/microsoft/skills) is Azure SDK and Foundry skills (174, per a search result), not Office. I found no Office-UI skill from Microsoft.
- **Autodesk:** announced MCP servers for Fusion in April 2026 (a search snippet; the page failed to load twice, so I have not read it). **Ableton, Steinberg, Blackmagic:** no official skill or MCP server found. For Ableton and Blender, every MCP server I found is community-made. I could not verify the "official Blender server" label on one aggregator (404).

So the gap our packs fill is real: skills for the **desktop app on the screen** are the part nobody ships. This also means the standard container will not break when vendors arrive, because they ship in it.

## 4. Naming, and what changes

**Everything a pack adds can live inside a standard Agent Skills folder.** Check against the spec:
- The skill is `SKILL.md` (name, description). `controls.md`, `keys.md`, `pitfalls.md`, `verify.md`, `overlay-*.md` and scripts are "additional files", allowed by the spec; the `SKILL.md` router points to them one level deep, as the spec advises.
- `pack.json` (changelog, `app.versions`, `ladder`, `verified`) is one more file. Another Agent Skills reader ignores it, and reads the skill whole. `metadata` can hold `version` as a string, for readers that want one. The spec's `compatibility` (500 characters) can say "Windows; Word 2019 or later".
- **Not in the standard, and AgentMux-only:** the verification records, the range-tag filtering by installed version (a reader that does not filter shows all the lines, with their tags, which is harmless), approval of scripts and the MCP pointer, the Packs-tab cards, and signing.

**Name.** Options, against the evidence:
- **"Computer-use skill"** — what the operator leans to. It matches the standard (it *is* a skill), says what it is for, and matches how Orca, Hermes and Cua already name theirs. Cost: long.
- **"App skill"** — short, and Figma and Adobe use "skills" for app work. Ambiguous with a skill *about* using an app's API (Adobe's), which is what most vendors ship. Good as a badge, not as the canonical name.
- **"Pack"** — no precedent in this field; its only close use is bundles ("pack" as a bundle). It hints at "more than a skill", but that "more" is a manifest file, not a new kind of thing.
- **"Plugin"** — the industry word for "skills plus MCP plus hooks" (Claude Code, Adobe, Figma, Cursor). Rejected here: AgentMux already uses *widget* and *extension*, and a Claude Code *plugin* means something else on the same machine.

**Recommendation:** call the unit a **computer-use skill**; use **App skill** as the short badge. Keep `pack.json` as the manifest filename (it is a file name, not a user concept), or rename it `computer-use.json` if you want the vocabulary to match everywhere; I would not spend a change on it.

**What changes in SPEC_PACKS** (`docs/specs/SPEC_PACKS_2026_11_05.md`, wording and one UI point; no new work):
1. §1 and the title: "A computer-use skill is an Agent Skill folder plus a manifest"; drop "pack" except as the old name, and say the folder is readable by any Agent Skills client.
2. §3: state that `metadata.version` and `compatibility` are filled in for other readers, and that `pack.json` is ignored by them.
3. §8, the **tab**: the spec has a separate Packs tab (Global | Personal | Skills | Packs | Bundles). If they are skills, the simpler UI is the **Skills** tab with an "App" filter and the app-specific card fields (installed version, verified at v, pass records). That would also remove the "A pack is a skill with a manifest" explanation. It is a UI change to something the operator approved, so I have not assumed it; if you prefer the separate tab, the name still works there ("Computer use").
4. §9: "bundle lists packs by reference" becomes "a bundle lists computer-use skills", which is how it already lists skills. The `.pack` export file stays: a folder zipped with the manifest, extension `.skill` as Adobe uses for its release asset, or `.pack`; choose when Stage 4 starts.
5. Examples folder `docs/examples/packs/` and the guide skill: rename in the same pass.

**What changes in the repo idea (#1629, the public `agentmuxai/skills` repo):** model it on Adobe's layout: one monorepo, `skills/<app>/` each a standard skill folder, per-skill semver and tags (`blender-v1.2.0`), a release asset per skill, and a root `.claude-plugin/marketplace.json` so Claude Code can also `/plugin marketplace add agentmuxai/…` it. The repo name follows the decision (`agentmuxai/computer-use-skills` if N1 is accepted). That makes the repo useful to any Agent Skills client, which is the point, and it keeps verification records out of the public repo (they carry host names; see the public-repo rule).

## What I could not settle

- CUA-Skill's code release and licence; UFO²'s latest commit date; the dates of the glincker, Adobe and Anthropic commits.
- Whether Microsoft or Autodesk publish an Office or Fusion UI skill (I found none; absence in search results is weak evidence).
- The Adobe Creative Cloud and Orca guides' contents, which sit behind a connector or inside a binary.
- Figma's in-app skill format (two help articles not read).
- Anything about OpenAI's equivalents beyond Codex reading `SKILL.md`.

# SPEC: A public computer-use skill repository, `agentmuxai/skills`, installed on demand

**Status:** active — approved by the operator 2026-11-05 ("yes proceed with computer use implementation", relayed by AgentY, with R2 to R6 as recommended: Apache-2.0, DCO, every listed skill signed, only the guide bundled, AgentY creates the repo); build #1630, first PR #6582. The repository is `agentmuxai/skills`.
**Date:** 2026-11-05
**Author:** Manoz@Area54, at AgentY's request after the operator asked (quoted): "we also need a pack repository, so we dont ship them all, in fact is there already a pack repository? we may want to start it as a public contribution on agentmuxai/skills, what do u think?"
**Written against:** origin/main `c655acddd` (v0.60.17).
**Addendum to:** `SPEC_PACKS_2026_11_05.md` (the skill format §3, trust §5, proof §7, versions §10, stages §14).
**Builds on:** `SPEC_WIDGET_SHARING_2026_10_10.md` §2 (signed packages) and §4 (the catalog), whose public repo [`agentmuxai/widgets`](https://github.com/agentmuxai/widgets) (Apache-2.0, public) is the working precedent.
**Informed by:** `REPORT_COMPUTER_USE_SKILLS_PRIOR_ART_AND_NAMING_2026_11_05.md` (#1631, approved): the name is "computer-use skill", it lives in Memory > Skills with an App filter, and Adobe's `adobe/skills` is the layout precedent.

## Decisions (approved 2026-11-05, as recommended)

| # | Decision | My recommendation |
|---|---|---|
| R1 | Is there already a skill repository, and do we start `agentmuxai/skills`? | None exists. Yes, start it, modelled on `agentmuxai/widgets` (§1). |
| R2 | Licence | **Apache-2.0 for everything** (§4). Not CC-BY-4.0 for text. |
| R3 | DCO or CLA | **DCO** (a `Signed-off-by` line), no CLA (§4). Needs your call; a CLA is only worth it if you may want to relicense later. |
| R4 | Must every listed skill be signed? | **Yes**, by the AgentMux publisher key or the author's registered key. The app shows which (§5). Unsigned skills are for local import only. |
| R5 | What does the app still ship? | **Only `write-a-skill`** (3.6 KB), so authoring works offline. Every app skill comes from the repo (§1). |
| R6 | Creating the repo | The operator, or `gh-agent sudo` (the build asks; I have not tried to create it). |

## 1. What exists, and the repo layout

**Today there is no skill repository.** The five Stage 1 skills and the guide are in the app repo under `docs/examples/skills/` ([#6549](https://github.com/agentmuxai/agentmux/pull/6549)). The widget catalog is the model: one public repo, one folder per item, a publishers file, CI that builds a signed `index.json` and a zip per item, published on GitHub Pages ([README](https://github.com/agentmuxai/widgets), `scripts/build.mjs`, `.github/workflows/catalog.yml`, `publishers.json`).

**Layout of `agentmuxai/skills`:**

```
skills/
  agentmux-word/        # one standard Agent Skills folder per skill
    SKILL.md  computer-use.json  controls.md  keys.md  pitfalls.md  verify.md
    overlay-<range>.md  test.json  scripts/  skill.sig  verified.json
  agentmux-blender/ ...
guide/write-a-skill/    # the guide (the one skill the app also ships)
publishers.json         # publisher -> public key, as in the widget catalog
verification/           # records from test machines (§6)
scripts/build.mjs       # validate, then write index.json and the zips
LICENSE  README.md  CONTRIBUTING.md  .github/CODEOWNERS  .github/workflows/
```

- **Folder name = skill name = `<publisher>-<id>`** (`agentmux-word`). The Agent Skills spec requires the folder to match `name`, and this keeps the repo readable by any Agent Skills client (`skills/` is where `anthropics/skills` and `figma/mcp-server-guide` keep theirs). It is also how skills from different publishers avoid a clash: two publishers can each have a `word`.
- **`computer-use.json` gains `publisher`** (`agentmux`); the skill's catalog id is `<publisher>.<id>` (`agentmux.word`), the same form as a widget id. This needs one new field in SPEC_PACKS §3; Stage 1 skills get it when they move.
- **`verified.json` is a new file, excluded from the content hash and the signature** (like `skill.sig` and, for widgets, `widget.sig`). SPEC_PACKS §7 puts the pass records in `computer-use.json`; that would change the skill's hash, and so break its signature, every time a test passes. The records move to `verified.json`; `changelog` stays in `computer-use.json` because it is part of the version.
- **Releases:** one git tag per skill version, `<publisher>-<id>-v<semver>` (`agentmux-word-v1.3.0`), the form `adobe/skills` uses. The version in `computer-use.json` is the source of truth; CI refuses a changed skill whose version did not increase. History of every version is the git history, which serves SPEC_PACKS §10.1 (diff and roll back) for catalog skills.
- **What moves:** the five skills and `write-a-skill` from `docs/examples/skills/` to the new repo in the build's first step, with `Manoz@Area54` run records replaced as in §6. `docs/examples/skills/` is then deleted in the app repo, leaving a one-line pointer in `docs/`.

**What the app still bundles: `write-a-skill` only.** It is a small text skill that a person needs before any network or catalog is available (and it is how a computer-use skill is written for a private app). It is seeded as a normal skill. Everything else is installed on demand; shipping five skills now and fifty later in the installer is the thing the operator asked to avoid.

## 2. In the app: Browse, install, update

Memory > Skills (SPEC_PACKS §8) gets a **Browse** view, built like Settings > Widgets > Browse the catalog (SPEC_WIDGET_SHARING §4.5), with the same code behind it where it can be shared.

- **srv:** `app_skills.catalog` fetches `index.json` and `index.json.sig` from `https://agentmuxai.github.io/skills/`, checks the signature against a **catalog key pinned in the source** (`PACK_CATALOG_KEYS`, a list so it can rotate), and lists each skill with its publisher's fingerprint, its verification summary (§6), what is installed here, and the app version found on this machine (SPEC_PACKS §8). `app_skills.catalog.install` fetches the index again, downloads the zip from the index's own origin (no redirects), and checks the zip's SHA-256, the skill's content hash, and its author's signature against the index entry, then adds it as a computer-use skill awaiting approval.
- **Own catalog key.** The skill catalog gets its own signing key, separate from the widget catalog's, so a leaked secret in one repository cannot sign the other's index. The shared part is code (`backend/widget/catalog.rs`'s fetch, signature and zip checks become a generic catalog client with two key lists); the keys and indexes are separate. This is "reuse the pipeline", not a second pipeline.
- **Approval is the widget flow** (SPEC_PACKS §5): text-only skills need nothing beyond adding the skill; `scripts/` and an `mcp` entry need the prompt with the publisher (signed, fingerprint, "the same key as your other agentmux packs"), each script with its hash, and the MCP pointer. Publisher pins work as for widgets: trust on first use, a `key_changed` warning, **Forget key** in Settings.
- **Update and pinning.** Browse shows "Update to 1.3.0" next to an installed older version. **Installing an update is always the person's click** (and an approval if scripts or the MCP pointer changed); the app never updates a pack by itself. "Follow latest or pin" (SPEC_PACKS §10.4) is about what an agent reference resolves to among the versions *installed here*: follow-latest tracks the newest installed version, a pin stays. A pack's `verified` badge for the newly installed version is shown before the click ("not yet tested at 1.3.0" is a normal state, §10.2).
- **Offline or catalog down:** Browse says so and shows what is installed; nothing else depends on the network.
- **Not a second path for trusted code:** the catalog lists skills only. It never installs a widget or an MCP server; the MCP pointer still goes through the MCP tab's own install and approval with a pinned version.

## 3. Phases

| Phase | Builds | Done when |
|---|---|---|
| **P1: the repo** | Create `agentmuxai/skills`; move the six folders; `publishers.json`, `build.mjs`, the Pages workflow, `CODEOWNERS`, `CONTRIBUTING.md`; sign the AgentMux skills | a pull request that breaks a rule fails CI; the index builds and signs; `npx skills add agentmuxai/skills` finds the folders (to be tried; not verified) |
| **P2: srv** | The generic catalog client, `app_skills.catalog` and `.install`, `PACK_CATALOG_KEYS`; `skill.sig` verification (SPEC_PACKS §5); publisher pins | a catalog skill installs as `needs_approval` with its fingerprint; a tampered zip is refused (tests) |
| **P3: Browse** | The Browse view, Install and Update, the verification summary, "From the AgentMux catalog" prompt line | the five skills install from Browse in a task dev build |

P2 and P3 need SPEC_PACKS stages 2 to 4 (skill storage, Memory > Skills, script and signature approval) to exist; P1 does not, and can start the day the spec is approved. The Stage 1 skills are readable as plain skills in the meantime.

## 4. Contributions

- **Licence: Apache-2.0 for the whole repo.** Same as `agentmuxai/widgets` and `adobe/skills`; one licence for text and scripts, with a patent grant, and no attribution banner a skill cannot show. CC-BY-4.0 for text would add a second licence and an attribution requirement a skill has no place to display. (Contrast: Anthropic's own document skills are source-available, not open; I read this in the repo's README and did not read their licence text. We want the opposite.)
- **DCO, not a CLA (recommended).** Contributors add `Signed-off-by` (`git commit -s`); a check enforces it. It adds no legal step for a person who wants to share a Blender skill. A CLA is needed only if the project may relicense contributions later; that is the operator's call (R3).
- **CI on every pull request** (`scripts/build.mjs`, no dependencies, like the widget catalog's), failing the PR on:
  - **Format:** `computer-use.json` (schema `computer-use-skill/v1`) and `test.json` validate; folder name = `SKILL.md` `name` = `<publisher>-<id>`; the Agent Skills limits (name 64, description 1024, `SKILL.md` under 500 lines); every file named in `computer-use.json` exists; script hashes in `scripts` match; the version increased if the skill changed.
  - **Signature:** `skill.sig` matches the files and the key `publishers.json` lists for that publisher (a different key for a known publisher fails).
  - **Secrets and typed passwords:** a scan flags (a) a `type` step or script that writes a value into a password, token or key field, (b) reads of credential stores (`cmdkey`, Windows Credential Manager, the Keychain `security` tool, `secret-tool`, browser profile databases, `.env` and `.aws` files), (c) literal tokens and private keys. A flag fails the check; a maintainer can clear a false positive with a recorded override in the PR. This is pattern matching, so it catches carelessness, not a determined author; that is why the next rule exists.
  - **Network and size:** a script that calls the network must say so in `computer-use.json` `scripts[].needs`, and the origin is shown in the prompt; per-file and per-skill size limits like the widget catalog's.
  - **Content rules (§7):** a licence-header scan for copied vendor text, and no image or icon files other than the skill's own.
- **Scripts need a human.** `CODEOWNERS` requires an AgentMux maintainer's review for any change under `scripts/`, any `mcp` entry, and `publishers.json`. A new network origin or a credential-store read gets a closer look, as with widgets. A text-only change needs one review.
- **Never merged on CI alone,** same as the widget catalog: "A maintainer reads every submission."

## 5. Who signs, and what the app shows

- **Computer-use skills AgentMux publishes** are signed with the AgentMux publisher key, the one already listed as `agentmux` in the widget catalog's `publishers.json`, so a person who has pinned `agentmux` for widgets sees "the same key as your other agentmux items". The private half is held outside the repository, as the widget catalog key is; this spec does not say where.
- **Community skills** are signed with their author's key, registered in `publishers.json` in the author's first PR (`npx -p @agentmuxai/widget-sdk agentmux-widget keygen` and `sign` already do this for widgets; the build adds a `skill` mode with the domain tag `agentmux-skill-sig-v1`).
- **Every listed skill is signed (R4).** The task suggested "unsigned or signed with their author's key"; I recommend requiring a signature for everything listed, because keygen is one command and an unsigned skill in a public index gives the person nothing to check. Unsigned skills still import from a `.skill` file or are written locally (SPEC_PACKS §9), shown "Not signed: AgentMux can't tell who made it".
- **In Browse and in the prompt:** a badge reads **AgentMux** (the AgentMux key), **Community · K7Q2-MZ4D-PX3A-9TWE** (a registered author key), or, for a local import, **Unsigned**. The catalog is "a second check, never an approval" (SPEC_WIDGET_SHARING §4.1).

## 6. Verification records

CI cannot run Word or Live (no desktop, no licence), and a green build must not suggest otherwise. The records come from machines that have the app, and are shown next to the skill.

- **Where:** `verification/<publisher>-<id>/<skill version>.json` in the repo, a list of `{ skillVersion, appVersion, platform, date, route, result: pass | fail, notes, evidenceSha256 }`. `build.mjs` folds a summary into each index entry, so Browse shows "Passed on Word 16.0 and Excel 16.0 · Windows · pack 1.2.0 · 2026-11-06" or "Not yet tested at 1.3.0".
- **Who adds them:** a test machine, by PR, after running the skill's `test.json` (SPEC_PACKS §7). `CODEOWNERS` requires a maintainer on `verification/`, so a contributor cannot self-certify their own skill.
- **Not copied from the local run as is.** The local `verified` records carry `runner: Manoz@Area54`. A public record says `runner: "AgentMux test machine"` (or `"community"` with a GitHub handle), with no host, user or path. The evidence is a SHA-256 of the screenshot, never the screenshot.
- **The installed copy keeps its own records** in `verified.json` (outside the hash), appended by a local run (SPEC_PACKS §7). Catalog records and local records show side by side: "passed elsewhere" and "passed here".
- **A record is not a guarantee.** It says a test passed on that app version on that day. A newer installed app is still **drift** (SPEC_PACKS §8, §10.3).

## 7. Content rules for a public repo

These go in `CONTRIBUTING.md` and are partly checked by CI (§4).

- **Our own words.** No copied vendor documentation, manuals, help pages or release notes. Facts (a shortcut, a COM class, an API name) are fine; the prose is the contributor's. A computer-use skill points to the vendor's documentation by link and says what to look for.
- **App names only to name the app,** in plain text ("Works with Ableton Live 12"). No vendor logos, icons, screenshots of vendor UI, or wording that suggests the vendor endorses the skill. Skill files use no vendor trademark as the skill's own name beyond naming the app.
- **Nothing about AgentMux's private systems:** no internal hostnames, machine names, paths, account or agent identifiers, infrastructure, logs, or unfixed weaknesses in AgentMux. A known, published vulnerability in a third-party tool may be named (the Blender skill names a CVE so the agent avoids that route); an undisclosed one may not.
- **No secrets, ever,** and no instruction that has an agent type a password (SPEC_PACKS §5: a computer-use skill tells the agent to ask the person).
- **Licensing of what a script uses:** a script may call an app's own scripting interface; it may not bundle the app, a vendor SDK, or a binary.

## 8. What this doesn't do

- No automatic installs or updates; no paid or rated skills.
- No trusted code from a computer-use skill: a computer-use skill grants nothing, and `Desktop` grants stay per app and per time (SPEC_PACKS §5).
- No skill for an app whose vendor's terms forbid automation: a maintainer checks the terms before listing one.
- No CI that runs the apps. Verification is by machines that have them (§6).

## 9. Open questions

1. **Rename.** If #1631's name is accepted, the repo becomes `agentmuxai/computer-use-skills` (or stays `skills` as a short URL); the decision is the operator's and changes only labels.
2. **`npx skills add agentmuxai/skills`.** I believe the `skills` CLI finds folders under `skills/` (the layout `anthropics/skills` and Figma use), but I did not verify which directories it scans. P1's done-when tests it.
3. **A Claude Code marketplace file.** Adding `.claude-plugin/marketplace.json` would let Claude Code users install skills as plugins (the way Adobe's repo does). That is useful but a second distribution path to keep signed and in step; I would add it after P3, if asked.
4. **Where the AgentMux publisher key is signed from.** CI cannot hold the key (a PR from a fork must not reach it), so AgentMux skills are signed by a maintainer's release step; how that step is run is for the platform lead, not this spec.

Agent: Manoz@Area54

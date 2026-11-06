# SPEC: Computer-use skills: per-application knowledge for computer use (Word, Excel, Ableton Live, WaveLab, Blender first)

**Status:** active — approved by the operator 2026-11-05 (v2, relayed by AgentY); build #1560. Renamed to "computer-use skill" on 2026-11-05 at the operator's word (see the next lines); the Stage 1 files are moving to `agentmuxai/skills` (task #1630). Stages 2 to 5 are not started.
**Operator decisions folded in (2026-11-05, quoted via AgentY):** "ok lets rename app packs simply packs. Ok, keep bundles, add another tab called packs .. we'll let our recent bundles work settle". So: the name is **computer-use skills** (UI, manifest, docs); Memory gets its own **Computer-use skills** tab; bundles stay as they are and computer-use skills are not folded into them.
**Renamed (2026-11-05, quoted via AgentY):** "look good, lets go with computer use skill. In that case itwould be part of the skills part of memory right?" So the name is **computer-use skill** (badge "App skill", never "plugin"); it lives in Memory > Skills with an App filter, not in a separate Packs tab (§8). That supersedes the tab and the word "packs" in the decision above; the rest (bundles untouched, not folded into them) stands. Files: manifest `computer-use.json`, signature `skill.sig`, export `.skill`.
**Date:** 2026-11-05
**Author:** Manoz@Area54, task #1559, at AgentY's request after the operator asked (quoted) "ok have him write it .. where in the UI would you suggest? toolchain?" and then "memory?"
**Written against:** origin/main `c09327ef7` (v2 of this draft; v1 was `1232ed81c`). #6539 was read while still open.
**Follows:** `docs/reports/REPORT_COMPUTER_USE_STATE_AND_APP_SKILL_PACKS_2026_11_05.md` (decisions D1 and D2 there).
**Builds on:** `SPEC_DESKTOP_DRIVING_2026_10_20.md` (the `Desktop` tool and its grants), `SPEC_AGENT_BUNDLE_FORMAT_V0_3_2026_10_05.md` (bundles), `SPEC_WIDGET_SHARING_2026_10_10.md` (signed packages, approval, widgets in bundles), `SPEC_ARMORY_PRELOADED_CREATIVE_MCP_CONNECTORS_2026_07_10.md` (the MCP catalog, Ableton already in it).

---

## 1. Summary

A **skill** is what an agent needs to be quick and safe in one desktop application: how to control it, ordered from the most reliable route to the least, its keyboard map, its known traps, and a check that proves each step worked. It is an [Agent Skill](https://agentskills.io/) (a `SKILL.md` folder) plus a small manifest. It is not a new engine: it rides on the skill catalog, bundles, the MCP catalog and the widget approval flow that exist today.

**Bundles are being reshaped (#21781: #6382 merged, #6539 about to merge, step 3 later), so computer-use skills do not depend on them:** a computer-use skill is its own thing with its own storage and its own file; a bundle can **list** the computer-use skills it gives an agent, because it is a skill, so a bundle lists it in its Skills group like any other (§9); only a copy of its files inside an exported `.abf` file waits for #21781 step 3. Nothing breaks if bundles change shape or go: a computer-use skill stays a skill in Memory > Skills.

Three rules shape everything below:

1. **A computer-use skill is knowledge. It grants nothing.** `Desktop`'s per-app grant, the blocklist, the rate limits and Stop apply exactly as before.
2. **A computer-use skill loads when the agent meets the app**, not for every agent at every launch.
3. **"Verified on Live 12.1" means a test passed on Live 12.1**, with the result recorded; it is never a claim from the skill's author alone.

## 2. What exists today (verified in the code)

- A catalog skill (`db_skills`) is **one text body** with `name`, `trigger`, `skill_type`, `description` and `content`. At an agent's launch, an `agent-skill` is written to `.claude/skills/<slug>/SKILL.md` (`agent_config.rs`), and a startup-file index lists each bound skill by name and description. A comment there already anticipates "a future scripts/ dir". There is **no place for sibling files** (`controls.md`, `keys.md`, scripts) today.
- Skills reach an agent three ways (`effective_skills`, `storage/skills.rs`): a direct **bind** to that agent, a **bundle** the agent has picked, or **reach** (`all_agents`, `new_agents`). A computer-use skill uses the direct bind and reach, so the bundle reshaping below does not touch its delivery. The skill-import flow (`skill_import.rs`) already reads a folder or HTTPS source, shows the text and a diff, and writes only what a person approves.
- A bundle carries instructions and skills (and widgets since #4650); it carries **no MCP servers** (#4475). **Bundles are mid-reshape** (task #21781, `REPORT_AGENT_BUNDLES_REFINE_OR_DROP_2026_11_03.md`, approved by the operator): #6382 (merged) stopped minting an empty bundle per agent and counts export and import use in `bundle-transport-use.jsonl`; #6539 (open, `m0047`) removes an agent's own bundle, so a bundle holds only what several agents share; the report's step 3 narrows a bundle to a reusable instruction set and stops `.abf` being the way to carry an agent, and says that if counted use is near zero, bundles can be dropped (its Option 3). This spec treats that as unsettled and does not build on bundle internals. MCP servers come from the MCP tab, whose catalog (`mcp-preload-catalog.ts`) already preloads Ableton.
- Widgets: a package has a content hash, an optional Ed25519 signature with trust-on-first-use publisher pinning, and an approval prompt naming every permission.
- `Desktop` is Windows only, pixels only (the report's §1). Its `apps` action lists windows with the executable name.

## 3. The format

A computer-use skill is a folder, shipped and stored as a catalog skill plus a manifest.

```
word/
  SKILL.md        # name, description, the ladder in four lines, pointers to the files below
  computer-use.json       # manifest (below)
  controls.md     # the ladder in detail: how to do each common job by the best route
  keys.md         # keyboard map, chords, the menu path for what has no key
  pitfalls.md     # known traps: modal dialogs, autosave, custom-drawn UI, slow operations
  verify.md       # the cue that proves each step worked (a title, a status text, a file on disk)
  test.json       # the skill's test task (§7)
  scripts/        # optional: a COM or script helper, run by the agent's shell, not by AgentMux
```

`SKILL.md` stays short (a router, under about 60 lines) so reading it costs little; the other files are read only when needed, which is the three-level loading Agent Skills already define.

**`computer-use.json`:**

| Field | Meaning |
|---|---|
| `schema` | `computer-use-skill/v1` |
| `publisher` | the publisher id (`agentmux`); the skill's catalog id is `<publisher>.<id>` (`agentmux.word`) and its folder `<publisher>-<id>` (`agentmux-word`), which is also the `SKILL.md` `name`, as Agent Skills requires |
| `id`, `name`, `version` | the skill's own id (`wavelab`), display name, and its version (semver) |
| `app.vendor` | e.g. Steinberg |
| `app.executables` | per platform: `{ "windows": ["WaveLab Pro 12.exe", "WaveLab Elements 12.exe"], "macos": [...], "linux": [] }` (matched like the `Desktop` grant matches an app: by executable name, never by title alone) |
| `app.titleHints` | window-title fragments, for apps hosted by a frame (Store apps) |
| `changelog` | the skill's own version history, by hand in Stage 1 (§10.1): version, date, who, what changed |
| `app.versions` | `supported` (a semver range, `null` while unknown) and `tested` (the app versions a test passed on) (§10.3) |
| (`verified`) | not in the manifest: pass records live beside it, in a `verified.json` that is left out of the content hash and signature, so a new pass does not break the signature (§7). In the public catalog they are `verification/<folder>/<version>.json` |
| `ladder` | the control surfaces, best first, each `{ kind: api \| mcp \| shortcuts \| desktop, note }`; the skill's own order, not a default |
| `mcp` | optional: the id of an entry in the MCP catalog (§6), never a command line |
| `scripts` | optional: each script with its SHA-256 and what it needs (a COM class, a socket port) |
| `platforms` | the platforms the skill claims; `Desktop` is Windows only today, so a macOS entry is knowledge only until `Desktop` reaches macOS |

**The ladder in every skill, in this order:** (1) the app's own API or scripting, (2) a plug-in or MCP server, (3) shortcuts and menu paths, (4) `Desktop` clicks, each followed by its verify cue. This is the order the `Desktop` tool already tells agents; the skill fills in the app-specific rungs.

## 4. Loading: when a computer-use skill enters an agent's context

Not preloaded for everyone. A computer-use skill costs nothing for an agent that never meets the app.

1. **By name.** The skill's `SKILL.md` description names the app and its executables ("Use when driving Word, WINWORD.EXE, a `.docx` document..."). An agent that has the skill bound sees the one-line description and loads the body when it matches, which is how skills already load.
2. **By window, for agents without it bound.** When `Desktop apps` lists a window whose executable matches a computer-use skill's `app.executables`, that window's row gets `skill: { id, name, verified_here }`, and the `look` answer for it repeats a one-line pointer. The agent then reads the skill with `Skills get` (the catalog read it already has). No grant is implied by the pointer.
3. **Reach.** Computer-use skills default to **no reach**. A person can bind a computer-use skill to an agent (a direct bind), include it in a bundle, or set `reach` for all agents; its card (§8) offers "Give to all agents" only with a note that it adds a description line to each agent's startup file.
4. **Reading siblings.** `Skills get` returns the `SKILL.md` body today. Reading `controls.md` and the others needs a small addition: `Skills get` with a `file` argument for a computer-use skill, and the launch step writing the files beside `SKILL.md` (its managed-file manifest already tracks paths, so cleanup of stale files works).

## 5. Trust

| Skill contains | Needs | Why |
|---|---|---|
| Text files only | no approval beyond adding the skill | it is instructions, like any skill |
| `scripts/` | approval on first install and on any change, like a widget: the prompt shows the publisher (signed, with the fingerprint) or "unsigned", each script and what it needs, with its hash | a script runs with the agent's own rights |
| An `mcp` entry | the MCP tab's own install and approval, and a **pinned version** | an MCP server (Ableton's runs arbitrary Python inside Live in one variant) executes code; the skill only points at the catalog entry |
| Anything | never a `Desktop` grant | the person grants per app, at the time, with the countdown |

The signing scheme is the widget one (`SPEC_WIDGET_SHARING` §2: `skill.sig`, Ed25519 over id, version and content hash, publisher pinned on first use). An update from a different publisher key than the pin is refused with a clear message. A computer-use skill can ask for nothing the `Desktop` grant did not already allow, so a hostile skill's worst effect is bad advice to an agent that is still gated, rate-limited and audited. The skill text is untrusted input to the agent like any file; `Desktop` still refuses a blocked app. It does **not** yet refuse a password field (the guard is Phase 2 of the Desktop spec and is not built), so every skill must tell the agent to ask the person for any password, never to type one.

## 6. The MCP pointer

A computer-use skill says "use catalog entry `ableton-live`" (the MCP catalog's id; its server package is `ableton-mcp`), not "run this command". The MCP tab installs and approves it as today; the pack's `verified` records say which server version they were tested with. If the entry is not installed, the pack's `SKILL.md` says to continue down the ladder, and Memory shows "plug-in not installed" (§8).

## 7. Proof: the test task and who runs it

Each skill ships `test.json`: a short task, the steps expected, and the verify cues, for example for Word: "open a new document, type this sentence, save as `skill-test.docx` in the agent's workspace, confirm the file exists and holds the sentence". The cues are checkable without a person (a file, a window title, a script result).

- **Who runs it:** an agent with `Desktop` enabled on a machine that has the app, with the person's per-app grant at that moment. Hosts with such apps are Area54 (Windows), starpower (macOS) and charlie (Linux); the platform testers (Clare, Maricon) and the host stewards are the natural runners, asked by task.
- **Where the result goes:** a record `{ skillVersion, appVersion, platform, date, runner, route, result: pass | fail, notes, evidenceSha256 }` in the skill's `verified.json`, which is outside the content hash and the signature (a new pass must not break a signed skill). Evidence is the SHA-256 of the final screenshot or output file, never the file. The record is local to the machine that ran it and travels with the skill, marked "verified elsewhere" on the receiving machine. In the public catalog the records are `verification/<folder>/<version>.json`, with `runner` set to `AgentMux test machine` or `community:<handle>`, never a host or user name (`SPEC_PACKS_REPOSITORY_2026_11_05.md` §6). The audit log (`desktop-audit.log`) already holds every action of the run.
- **What "verified on Live 12.1" means:** at least one `pass` whose `appVersion` is 12.1.x on that platform. A newer installed version than any verified one shows as **drift** (§8), the same idea as a provider CLI behind its pin.
- **Not claimed:** a computer-use skill with no `pass` is shown "unverified". Nothing blocks using it.

## 8. The UI: computer-use skills live in Memory > Skills, with an "App" filter

AgentY suggested Toolchain; the operator then asked "memory?", chose a separate Packs tab, and on 2026-11-05 settled it (quoted via AgentY): "look good, lets go with computer use skill. In that case itwould be part of the skills part of memory right?" Yes. A computer-use skill **is** a skill with a manifest, so it shows where skills show: **Memory > Skills**, with an **App** badge and an **App skills** filter. There is no separate tab and no new `MEMORY_SECTIONS` item.

- **Toolchain** lists what **AgentMux itself runs**: provider CLIs, Node, git and similar. Showing Word or WaveLab there makes it a general application inventory and needs a new installed-app scanner. Toolchain gets nothing in v1.
- **Memory > Skills** is where agent knowledge already lives, and the skill editor, import, reach and bundle membership already work on it.

**What the list shows.** The Skills tab's cards (the kit's `Card` and `CardGrid`, `frontend/app/element/ui/`; the style Skills and Bundles use) get an **App skill** badge for a skill with a `computer-use.json`, and a filter chip row: **All | App skills**. An App skill card adds, through the kit's existing slots: `icon` the app's icon (or a generic one), `subtitle` the vendor and installed version, `status` the dot (ok = verified on the installed version, warn = drift, muted = not found or unverified), `meta` the short facts ("verified 12.1 Windows", "route: COM", "Desktop: not granted"), `actions` on hover (Run test, Install plug-in), `attention` for a failed test. Selecting a card opens its detail below its row (`CardGrid`'s `renderDetail`). No custom card CSS: if one needs something the kit lacks, it is added to the kit.

```
Memory   Global | Personal | Skills | Bundles
------------------------------------------------------------------------------
 Skills   [ All | App skills ]                  [ Browse ] [ Import ] [ New ]
 ----------------------------------------------------------------------------
 +------------------------+ +------------------------+ +---------------------+
 | [App skill] Word       | | [App skill] Ableton    | | [App skill] WaveLab |
 | installed 16.0.18      | | installed 12.2         | | not found here      |
 | verified 16.0   [ok]   | | verified 12.1  [drift] | | verified 12.0       |
 | route: COM (plug-in ok)| | route: Remote Script   | |   (elsewhere)       |
 | Desktop: not granted   | |   MCP (not installed)  | | route: COM scripts  |
 | [ Run test ]           | | [ Install plug-in ]    | | [ Open ]            |
 +------------------------+ +------------------------+ +---------------------+
 ----------------------------------------------------------------------------
 detail, opened under the card's row:  Ableton Live  v1.2   publisher: AgentMux (signed K7Q2-MZ4D-PX3A-9TWE)
   files: SKILL.md controls.md keys.md pitfalls.md verify.md   [ Edit ]
   verified: 12.1 Windows pass 2026-11-09   12.0 macOS pass (elsewhere)
   given to: 2 agents      [ Give to an agent... ]  [ Export ]
```

- **Installed version** comes from the executable's file version on this machine (found by the executable names in `computer-use.json`; a lookup per row, no inventory scan). "Not found" is a normal state.
- **Drift** compares the installed version with the newest `pass` for this platform, with the same wording style as provider CLI drift.
- **Desktop grant** is shown read-only with a link to Settings; the switch and the grants stay where they are.
- **Plug-in state** is the MCP entry's state from the MCP tab.
- **Run test** starts the skill's test task as a normal task for an agent (it does not run it itself), so the grant prompt and the audit trail are the usual ones.
- **Browse** opens the public catalog (`agentmuxai/skills`, `SPEC_PACKS_REPOSITORY_2026_11_05.md`), filtered to computer-use skills, with install and update on demand; the app ships only the "Write a computer-use skill" guide.
- Card states to draw and screenshot: installed and verified, drift, not found, unverified, plug-in missing, and the empty filter (with "Browse" and "New").

**Alternatives rejected:**

| Alternative | Why not |
|---|---|
| An "Apps" section in Toolchain (AgentY's suggestion) | An inventory of the user's apps is not Toolchain's job, needs a scanner, and splits a skill across two panes |
| A separate Packs tab (the operator's earlier choice) | Dropped by the operator once the name became "computer-use skill": it is a skill, so it lives with skills |
| A new top-level "Apps" pane | One more pane for a handful of skills; discoverability is no better than Memory |
| Settings > Desktop control | That page is for the consent switches; a skill library is not a setting |
| In the agent's own pane, as a chip | Per-agent, so each agent would show different data about the same skill |

**Visual checks before the build merges:** the macOS and Linux look checks, as for any pane change, plus a screenshot of each card state above.

## 9. Authoring and sharing

- **Draft.** An agent writes the skill from the vendor's documentation and a recorded exploration. There is no macro recorder (`Desktop` records no replay, spec §4); the "recording" is the agent's own notes of steps it verified, plus the `desktop-audit.log` lines of that session. A guide skill, "Write a computer-use skill", tells the agent the format, the ladder, and to put only steps it saw succeed into `controls.md` and `verify.md`.
- **Review.** A person reads the skill in Memory before it is saved, with the diff view the skill importer already uses ("shows the whole text and a diff, writes only what a person approved").
- **Share, as a file.** A computer-use skill is **self-contained**: its own folder (`computer-use.json` plus the files in §3, with `skill.sig` if signed), exported and imported on its own as one file (a zip with the extension `.skill`) from Memory > Skills ("Export", "Import"), installed from the public catalog (Browse, `SPEC_PACKS_REPOSITORY_2026_11_05.md`), and sent to a LAN peer with `Message` files. The receiving machine's import shows the skill, its scripts and its MCP pointer for approval, as widgets do (installing offers, never approves).
- **In a bundle, by reference** (operator, 2026-11-05: "since bundles are made of instructions, memory, and skills ... I suppose packs will also become a part of a bundle?"). Yes, and it needs nothing new: a computer-use skill is a skill, so a bundle lists it in its **Skills** group through the existing `db_bundle_skills_ref`, by reference, not a copy. An agent that picks the bundle gets the skill bound to it (the same delivery as a direct bind, §4), and a computer-use skill can sit in several bundles while staying one skill with one set of verified records. The bundle detail (category card groups, Maop's task) shows it as an App skill card in the Skills group, with the existing "Add skill" and "Remove from bundle". Removing it from a bundle does not remove it from agents that have it by a direct bind.
- **Inside an exported `.abf`, as a copy.** An exported bundle file cannot hold a reference to a skill on another machine, so a skill in it has to travel as a copy, and a computer-use skill is a folder of files where a skill today is one text body. Carrying the folder (under an optional key; whether older importers ignore unknown keys is to be checked in `bundle_import.rs`) and importing it as a real skill with its own approval is the one piece that waits for #21781 step 3, because step 3 may change the `.abf` import path. The reference form above and the `.skill` file do not wait.
- **Version.** `skill.version` is semver; the importer keeps both versions until the person chooses, as it does for skill updates.

## 10. Versions

The operator: "packs would also need to keep track of version", and, on apps that change between releases (their example, DaVinci Resolve 20 and 21): "we want a best practice method to manage that". Four different things carry a version; keeping them apart is most of the method.

| Version of | Example | Kept where |
|---|---|---|
| The **skill** | `word` 1.3.0 | `computer-use.json` `version`, its history (§10.1) |
| The **app** it was written for and tested on | Live 12.1, Blender 5.2.3 | `app.versions` and each `verified` record (§10.2, §10.3) |
| A **plug-in or MCP server** it points at | `ableton-mcp` 0.2.1 | the MCP catalog entry's pinned version (§5, §6) |
| The **skill an agent got** at launch | `word` 1.3.0 | the agent's launch record (§10.4) |

### 10.1 A computer-use skill's history

Every saved change to a computer-use skill is a **version**, kept. The skill's `version` is semver (patch for wording, minor for a new step or route, major for a rewrite of the ladder). The detail of a card lists the versions with who and when; any two can be **diffed**; **roll back** makes a new version that restores an old version's content, so history is never rewritten. This is the idea Personal Memory's history already has, and the Computer-use skills card shows only the current version.

Stage 1 keeps this by hand in `computer-use.json` `changelog` (version, date, who, what changed). Stage 2 stores the versions as rows, the way skills and bundles already keep versions.

### 10.2 Verification belongs to a computer-use skill version

A `verified` record names **both** versions: `{ skillVersion, appVersion, platform, date, runner, route, result, notes }`. After an edit the card shows **"not yet tested at v1.3"** until the test runs again; the earlier passes stay in the list, each marked with the skill version it covers ("passed at v1.2, Live 12.1"). The claim "verified on Live 12.1" counts only a pass whose `skillVersion` is the current one. A person may **carry a pass over** to a text-only edit; that is recorded by name, not assumed.

### 10.3 App versions: one method

**One skill per app, never a fork per version. A version-independent base, small per-version overlays holding only the differences, a range tag on each fact that differs, feature detection before any version check, and a stated rule for a version nobody has tested.** The parts:

1. **Base plus overlays.** `SKILL.md` holds what is true for every version: concepts, workflows, the ladder. A difference goes in a short `overlay-<range>.md` (`overlay-21.md`), or inline as one line with a tag. Nothing is copied: a fact lives in one place, so a fix is made once.
2. **Range tags.** A line that differs starts with `[v <range>]`, for example `[v >=21]`, `[v <21]`, `[v >=20.1 <21]`; an untagged line is true everywhere. A tag says only when it is true. A range seen but not pinned down uses `≈` ("seen on 21.0.1, may hold earlier"). From Stage 2 the loader gives the agent only the lines true for the **installed** version (found from the executable, §8); in Stage 1 the agent reads the tags itself.
3. **Feature detection first.** Where the app has an API, the skill tells the agent to check that a call, property, menu item or command exists, and to read warnings, **before** relying on the version number; the tag is the fallback and the explanation. (Blender's skill does this: `Material.use_nodes` is deprecated in 5.2, so it checks before setting it.)
4. **An unknown newer version.** The loader uses the nearest older overlay, says "this skill was tested up to 20.1; you have 21.2: untested", and the skill's rule is: verify after every step. It never refuses a newer version outright.
5. **A new release.** An agent reads the vendor's release notes, drafts the overlay, and runs the skill's `test.json` on the new version. A failing step points at what changed. A person reviews the overlay before it is saved, and the passes are recorded against the new app version.
6. **Learned in use.** When an agent meets a difference during real work, it adds a **proposal** ("seen on 21.0.1: the Deliver page moved") to the skill's proposals list for review. It never edits the skill silently.

**Why this method, against the outside patterns:**

| Pattern | What it does | What we take, what we change |
|---|---|---|
| **MDN browser-compat-data** ([schema](https://github.com/mdn/browser-compat-data/blob/main/schemas/compat-data-schema.md)) | One entry per feature, with `version_added` and `version_removed` per browser, `≤` where the exact release is unknown, support listed as an array "most relevant and general first", notes and flags | Range data per fact, not a copy per version; `≤`-style honesty about what is known (our `≈`); the general entry first, specific ones after. We keep the data as tagged prose, because the reader is an agent reading text, not a table renderer |
| **Feature detection over version sniffing** ([MDN](https://developer.mozilla.org/en-US/docs/Web/HTTP/Guides/Browser_detection_using_the_user_agent)) | Test whether the capability exists; sniffing is "very hard to do reliably", you cannot assume a feature from a version string, and it is the last resort in MDN's order | Rule 3: check first, tag second. A version number tells the agent what to expect; only the app can tell it what is there |
| **Compatibility ranges** (VS Code `engines.vscode`, [docs](https://code.visualstudio.com/api/working-with-extensions/publishing-extension): a semver range, the marketplace picks the highest compatible version) | A declared range decides whether the extension is offered | `app.versions.supported` is the same kind of range. **We differ on a newer app:** VS Code withholds the extension; a computer-use skill still helps on an untested version, so we load the nearest older overlay with a warning (rule 4) |
| **A fork per version** (one skill for Live 11, one for 12) | Simple to read | Rejected: every shared fact is copied N times and drifts, tests multiply, and a fix is made N times. The overlay holds only the delta |
| **A version table the agent must check first** | Looks safe | Rejected: false certainty, the failure MDN names for sniffing; it also breaks on the first release nobody listed |

### 10.4 References: latest or pinned

A direct bind, a bundle's list and `reach` all point at a computer-use skill by id. **The default is "follow the latest version";** a pin is optional (a person pins a version they have tested or approved, and a computer-use skill with a script is pinned by the approval itself, §5). When a pinned skill has a newer version the card says so. At launch, the agent's launch record notes which skill version it was given (Stage 2).

### 10.5 Import and sharing

A received `.skill` with a **newer** version of a computer-use skill already present shows the **diff** before it replaces anything; both versions are kept until the person chooses, as the skill importer does. An older version arriving does not downgrade silently. A different publisher key than the pin is refused (§5).

### 10.6 Worked example: DaVinci Resolve 20 and 21

No Resolve skill is built in Stage 1; this shows the method. The differences below come from press coverage of Resolve 21 and 21.1 ([RedShark News](https://www.redsharknews.com/davinci-resolve-21.1-new-features-release), [No Film School](https://nofilmschool.com/davinci-resolve-update-21-1), [Motion Media](https://www.motionmedia.com/mm-blog/davinci-resolve-21-1-update-new-features)); **Blackmagic's own release notes are the source to read before writing the skill**, and none of this is verified by a test.

- Resolve 21 requires **Resolve Studio** for advanced (Python) scripting, and Python 2 support is gone; 21.1 adds about 20 scripting APIs and a built-in console, and 21.1 adds MCP support for AI assistants.
- **Base `SKILL.md`** (all versions): the ladder (scripting API, then an MCP route, then shortcuts, then `Desktop`), the Cut, Color and Deliver workflows, the verify cues (the render queue shows the job; the output file exists).
- **`[v >=21]`** scripting needs Studio: first check that the scripting object is reachable; on the free edition, skip rung 1 and say so.
- **`[v <21]`** scripts may be Python 2 or 3 and run on the free edition; **`[v >=21]`** Python 3 only.
- **`[v >=21.1]`** a built-in MCP route and about 20 more scripting calls exist: check that a call exists with `hasattr`-style detection before using it (rule 3).
- **On 21.2 (untested):** the loader uses the 21.1 overlay, tells the agent "tested up to 21.1", and the agent verifies each step. If the Deliver page differs, the agent adds a proposal ("seen on 21.2: ...") and a person reviews it.
- **When 22 ships:** an agent reads the notes, drafts `overlay-22.md`, runs `test.json` on 22; a failing step marks what moved.

### 10.7 What lands in which stage

| Stage | Versions |
|---|---|
| 1 | Skill `version` and `changelog` in `computer-use.json`; `verified` records with `skillVersion`; `app.versions` (`supported`, `tested`); the range-tag syntax and rule 3 in the computer-use skills themselves, read by the agent |
| 2 | Version rows (history, diff, roll back); verification stored by skill version; the loader that filters tagged lines by installed version and warns on an untested one; the launch record |
| 3 | The card's version, "not yet tested at v1.3", the version list, diff and roll back in the detail; the proposals list for review |
| 4 | Latest or pinned references; the import diff for a newer `.skill` |

## 11. The first five computer-use skills

From the report's table, plus Excel and Blender added by the operator (2026-11-05, "lets add blender and excel"). Each ships with its test. Blender is free and cross-platform and its route needs no `Desktop`, so its test already runs, and passed, on Windows (Blender 5.2.3); the same script route can be run on charlie and starpower.

| Skill | Best route (rung 1 and 2) | Then | Test |
|---|---|---|---|
| **Word** | COM through a Word MCP server (open, edit text and tables, save, export to PDF); a plain `.docx` edit when the file is closed | Office.js for cloud documents; shortcuts | New document, type a sentence, save `skill-test.docx`, check the file |
| **Ableton Live** | A Remote Script with an MCP server or OSC bridge (tracks, clips, parameters); Ableton is already in the MCP catalog | Keyboard map for Session and Arrangement; `Desktop` only for the browser and plug-in windows | Create a MIDI track and a clip with one note; read it back through the API |
| **WaveLab** | Its COM/ActiveX scripting (VBScript or JScript) and its built-in ECMAScript scripts, for batch and file work | Shortcuts and the Master Section; `Desktop` clicks with the skill's landmarks for meters and clip editing | Open a WAV, normalise it, save a copy, check the copy's level with a script |
| **Excel** | COM (`Excel.Application`) for an open workbook or a calculated result; openpyxl or XlsxWriter for a closed `.xlsx` (openpyxl does not calculate formulas) | Shortcuts (`Ctrl+G`, `F2`, `Alt+Q`); `Desktop` with zoom | New workbook, 1 2 3 in A1:A3, `=SUM(A1:A3)` in B1, save, read 6 back, also from the closed file |
| **Blender** | Its Python API run headless: `blender --background --python` (bpy); a pinned BlenderMCP only behind the skill-approval rule (it has a published code-injection flaw, CVE-2026-10688) | Shortcuts; `Desktop` with zoom | Add a cube, red material, render one 160 by 120 frame, check the PNG; **runs on any platform without `Desktop`** |

## 12. Out of scope, and how computer-use skills improve later

Separate builds (the report's D3 and D4):
- **The `Desktop` tool's UI Automation `tree`, `drag` and `zoom`.** Today rung 4 is "click at a pixel position and look". When `tree` lands, `controls.md` lists **control names and roles** instead of coordinates, and a verify cue can be "the status text reads X" from the tree. `drag` and `zoom` make sliders, faders and meters reachable. `computer-use.json` reserves `landmarks` (name, role, hint per control) so a computer-use skill written now can be upgraded without a new format.
- **Driving another machine** (D4). A computer-use skill is knowledge and works wherever its agent runs; a remote `Desktop` would use the target machine's grants and its own verified records.
- macOS and Linux input for `Desktop`. Computer-use skills may claim those platforms as knowledge only until then.

## 13. Open questions

1. **Sibling files and `Skills get` `file`** (§4) are the one backend change a computer-use skill needs beyond data. Alternative: a v1 with everything in one `SKILL.md` (no sibling files, longer text). I recommend sibling files; the one-file form is the fallback if the launch-time writer is hard to extend.
2. **A bundle listing computer-use skills** (§9): needs nothing new, because they are skills. Only the copy of a skill's folder inside an exported `.abf` waits for #21781 step 3, which may change the `.abf` import path; whether older importers ignore unknown keys is to be checked in `bundle_import.rs`. Read #6539 and the step 3 PR before that part.
## 14. Stages and rough cost

My estimates, not measured.

| Stage | What | Cost |
|---|---|---|
| 1 | Skill format, `computer-use.json` (with `changelog`, `app.versions`, `verified` by skill version), the "Write a computer-use skill" guide skill, **five** computer-use skills (Word, Excel, Ableton Live, WaveLab, Blender) written as plain skills with all text in `SKILL.md`, the range-tag syntax; tests run by hand where the app exists (Blender done; Word, Excel, Ableton, WaveLab need the app installed and `Desktop` on) | 4 to 6 days, no Rust |
| 2 | Backend: `db_app_skills` manifest, sibling-file writing at launch, `Skills get` `file`, the `skill` field in `Desktop apps`, verified-record append | 1 week, Rust, with tests |
| 3 | Memory > Skills: the App badge and filter, App skill cards (drift, detail, Run test, Give to), with the macOS and Linux look checks and a screenshot of each card state; **Browse** with `SPEC_PACKS_REPOSITORY_2026_11_05.md` P2 and P3 | 1 week, TypeScript |
| 4 | Script and `skill.sig` approval; the standalone `.skill` export and import | 1 week |
| 5 | A copy of listed computer-use skills inside an exported `.abf`, after #21781 step 3 settles | a few days |

Stage 1 proves the value before any code is written; stages 2 to 5 follow only if it does. The operator approves in conversation; AgentY tells me, I complete this task, and the build starts.

Agent: Manoz@Area54

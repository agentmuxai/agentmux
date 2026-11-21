# Spec: website and docs media refresh: crisp screenshots and workflow videos, shot together with the video series

**Status:** draft

Date: 2026-11-20 · Task: Spec #48240, build #48241 · Author: Loap. The capture and export standard (§5) is approved by the operator; the workflow list (§3) is not yet, and the other decisions (§10) wait for the operator.
Written against agentmux `09ad1d53c6`, agentmux-docs `84aa5fe` and the landing site at `2dac5aa` (all current main on 2026-11-20).

This is the one plan that joins what already exists: the demo-workspace seed spec (#30225), the getting-started video series spec, the landing re-shoot (#29493), the docs screenshot coverage report and the docs-screenshot audit (#654). It does not start over and does not audit prose; it takes the audit's list as the list of docs pages whose pictures change.

The request, restated as requirements:

1. The docs carry a lot of old pictures; the ones that show a UI that no longer exists are retaken.
2. The website (landing first, docs after) gets a new set of screenshots and videos that show **workflows** (what a person gets done with several agents), not single panes.
3. Every image is **crisp**: none is shown larger than the pixels it has. Videos are shot with the video series, from the same demo data.

---

## 1. Short answer

- **Why the pictures look upscaled.** Three causes, in order of weight (§4): every screenshot is captured at one device pixel per CSS pixel (so on a 2× or 3× screen it is enlarged by the browser); the landing page stretches each image to the width of its column with no 2× file offered, so a 560 or 800 px picture is drawn wider than it is; and the docs show 1280 px wide 1× pictures in a column of about 720 px with no `srcset`, which is sharp on a 1× screen and soft on a 2× one. The videos are 800 px wide clips shown in a larger column.
- **The fix is a capture standard plus a check that enforces it** (§5): capture at device scale factor 2 on every host, keep a lossless master, publish 1× and 2× web files with `srcset`, record at 1080p minimum from a 2× capture, and a script that fails any image or clip that is under 2× its slot or captured below scale 2. **All 104 raster images in the docs and all 10 images and the clip on the landing page fail it today** (§4.3).
- **The workflows** (§3): the first five for the landing page, then the series. The operator has not supplied a brainstorm list that is on record, so §3 is a table the operator fills or corrects; nothing is invented beyond the examples in the request and the series spec.
- **One pipeline** (§6): the demo seed (#30225) plays the story; one command captures stills and clips from it at 2×; redaction and the frame scan run on every output; one command publishes to the docs, the landing site and the media repository.
- **Order** (§8): crisp standard and check, then the seed's `landing` profile, then release 1 (5 landing shots and 1 clip), then the stale docs pictures, then the series.

---

## 2. What exists today (A)

| Piece | What it is | State |
|---|---|---|
| `scripts/ui-screenshots/` | `capture.mjs` (CDP page screenshots by suite: widgets, chrome, agent, memory, landing, sysinfo), `record.mjs` (page screencast, GIF / WebP / MP4), `publish.mjs` and `publish-landing.mjs`, `refresh.mjs` (all of it in order), `redact.mjs`, `demo-env.mjs` (the made-up `acme-web` project), `install-fake-claude.mjs` (a scripted agent: no account, model or network) | Works; every output is captured at scale 1 (`sizes.mjs`: small 800×600, medium 1280×800, large 1920×1080, each `scale: 1`) |
| Demo-workspace seed (#30225) | Profiles `empty`, `one-agent`, `three-agents`, `landing`, `kanban-story`; commands to seed, reset, step and verify; one story with six agents, a full board, jekts between agents, memory, a skill, a scheduled task; nothing real in it | Spec waiting for approval; not built |
| Getting-started series spec (Maop) | Nine episodes, E01 to E09, about 60 s each; page-only capture over CDP at a 1280×720 viewport and device scale factor 2 (2560×1440 frames), cut to 1920×1080 at 30 fps; a local voice; captions from the same timeline; redaction, a page-text check and an OCR frame scan on every cut | Draft spec; a working pilot on the macOS testbed |
| Landing re-shoot (#29493, Agent1) | 11 stills and 1 clip with fixed sizes (`ui-swarm` 1280×800, `ui-agent` 960×704, `ui-memory` 1100×704, `workspace` 1280×800) taken on a Windows capture host; assets are swapped into the landing site by its owner | In progress |
| Docs coverage report (2026-10-10) | 190 shots across 21 page groups, 8 parts, the tooling gaps | Part 1 and parts of 2 landed (the 104 images below) |
| Docs screenshot audit (#654) | 125 files inventoried, about 55 viewed: which images show a removed or renamed UI, with page, last-changed date and a confidence per image; 19 files are unreferenced | Input to §7 |

**What is missing**, in order of what it blocks:

1. **Content.** The seed is not built, so there is no busy, believable state (agents mid-work, a board with cards in every column, messages between agents). Every workflow below needs it.
2. **A 2× default.** The tooling can set a device scale (`sizes.mjs` has the field) but every size says 1, the recorder defaults to an 800 px width, and nothing checks.
3. **A check.** Nothing fails an image for being under-resolved or captured at scale 1.
4. **Two files per image on the sites.** Neither site offers a 1× and a 2× file.
5. **A pipeline that carries one set of demo data to stills and video.** The series spec assumes the seed; the seed assumes the series' start states; neither is built.
6. **The workflow scenes themselves** (§3): a card moving across the board, a message between two agents, a PR opened, a spec approved then built, a container agent. The seed has the story's agents and board; the scenes that play it for a camera are new work.

---

## 3. The workflows (B)

The first table is the only list on record (the series spec's episodes, each a distinct task). The second is what the request named as examples (an agent takes a card, messages another, opens a PR, the card moves; a Swarm following several agents; a spec approved then built; a container agent). **The operator is asked to confirm, strike or add to this list (decision 2); no row is final.**

### 3.1 Series episodes (Maop's spec, §2 of that spec)

| # | Title | One sentence | Length | Viewer learns | On screen | Used by |
|---|---|---|---|---|---|---|
| E01 | Install and meet your first agent | From download to an agent answering | 60 s | The ask, watch, read loop | Empty workspace; New Agent; the agent pane | Series; landing "start" section |
| E02 | Connect your Claude or Codex plan | Use the plan you already pay for | 60 s | Accounts, one click, per-agent account | Settings, Connectors, a made-up account row | Series; docs Connectors |
| E03 | Watch an agent work: answer, redirect, stop | You stay in control while it runs | 60 s | Questions wait for you; redirect and stop | A question card, a stop, the transcript | Series; landing "you stay in the loop" |
| E04 | Run several agents with Swarm | One message, many agents, nothing hidden | 60 s | Broadcast, rows filling in, open a result | Three agents; Swarm; Broadcast | Series; landing Swarm section; docs Swarm |
| E05 | An agent in a browser pane | Watch it read and click a real page | 60 s | The agent's pane, hand-off, its own profile | A local demo site in a browser pane | Series; docs Browser |
| E06 | The Vault: used, never read | Give access without giving the key | 60 s | Request, approval card, audit row | Vault pane; a fake secret | Series; docs Vault |
| E07 | Run any model | Switch an agent's model in the composer | 60 s | The model panel, a route, switching back | Composer; a real model answer | Series |
| E08 | Showcase: a research sweep | A leader and browser workers | 90 to 120 s | Many agents at once, you steering | Leader, five browser panes, Swarm, a table | Series; landing hero candidate |
| E09 | Showcase: change the brief mid-run | Redirect a whole team | 60 s | The leader passes the change on | As E08 | Series (optional) |

### 3.2 Product workflows named in the request (proposed, to be confirmed)

| # | Title | One sentence | Length | Viewer learns | On screen | Used by |
|---|---|---|---|---|---|---|
| W1 | A card becomes a PR | An agent takes a Kanban card, messages a colleague, opens a PR, the card moves to Review | 25 to 40 s clip + 3 stills | The board and the agents are one system | Kanban (card moves), the agent pane, Messages panel, a PR link | Landing (first five), docs Kanban |
| W2 | One message, several agents | A Swarm following several agents at once | 20 to 30 s clip + 1 still | Where every agent is, without opening a pane | Swarm, Broadcast, rows changing | Landing (first five); same as E04 |
| W3 | A spec, then a build | A spec is approved and the build task starts by itself | 30 s clip + 2 stills | Work flows from spec to build with a person's "looks good" | Kanban cards (spec, build), the agent that starts it | Landing (first five), docs Kanban |
| W4 | A container agent | An agent in a container does what a local one does, away from the host | 25 s clip + 1 still | Isolation without a different workflow | Agent picker runtime choice, the container badge, a pane working | Landing (first five) |
| W5 | The agent asks, you answer | A question waits for the person and the work goes on | 20 s clip + 1 still | You decide, it carries on | The question card, the answer, the next step | Landing (first five); same as E03 |
| W6 | A job that runs by itself | A scheduled job and a Drone graph run, and the Jobs tab shows what ran | 30 s clip + 2 stills | Automation you can see | Jobs tab, a drone graph, a run | Landing (later); docs Jobs |
| W7 | Memory every agent starts with | Global Memory and a skill reach every new agent | 20 s + 2 stills | Teach it once | Memory pane, a new agent using it | Docs Memory |

**First five for the landing page** (recommended): W1, W2, W3, W4, W5, each one still plus, for W1, the one release-1 video. The series then takes E01 to E09 in Maop's order (his decision 3 starts with E01, E03, E04).

**What each needs from the seed** (the `landing` and `kanban-story` profiles of #30225):

| Workflow | Needs |
|---|---|
| W1 | `kanban-story`: a board with cards in several columns, a stepped timeline (a card is claimed, a jekt is sent, a PR link appears, the card moves), the Messages panel filled by a delivered jekt |
| W2, E04 | `three-agents` or `landing`: agents with scripted turns of several steps, Broadcast |
| W3 | Two linked cards (spec, build) and the agent that holds the build; a "complete the spec" step that starts it |
| W4 | One container agent in the roster, or a recorded container pane; a Docker-less capture host needs the greyed state instead, so W4 is taken on a host with a container runtime |
| W5, E03 | A scripted turn that raises a question and a long turn that can be stopped |
| E05, E06, E07, E08 | The local demo sites, a fake Vault entry, a real model route (series spec §3) |

---

## 4. Why the images look upscaled (C)

### 4.1 What was measured

Pixel sizes were read from the files in the current docs and landing clones (header bytes; no image was opened or re-encoded).

**Docs** (104 raster images under `src/assets` and `public`):

| Folder | Files | Sizes (px) | Bytes |
|---|---:|---|---:|
| `widgets/` | 43 | 28 at 1280×744, 15 at 800×522 | 2.3 MB |
| `chrome/` | 23 | 3 at 1280×800; the rest crops from 128×267 to 560×480 | 0.9 MB |
| `settings/` | 11 | 9 at 1280×744, 2 at 1200×738 | 0.9 MB |
| `memory/` | 15 | 14 at 1100×704, 1 at 1200×738 | 0.66 MB |
| `agent/` | 8 | 5 at 960×704, others 1280×800, 1290×772, 596×738 | 0.75 MB |
| other | 4 | 800×800, 800×500 (a recording poster), 180×180, 32×32 icons | 0.1 MB |

**Landing** (10 images and 1 clip): 1280×800 (2), 1100×704 (2), 960×704 (3), 800×544 (1), 800×500 (1 poster), 560×480 (1); clip 800 px wide.

### 4.2 The causes

| # | Cause | Evidence | Effect |
|---|---|---|---|
| 1 | **Captured at 1× scale.** Every size in `sizes.mjs` has `scale: 1`; the viewport is overridden to a CSS size and the PNG has one pixel per CSS pixel | The pixel sizes above are the CSS sizes of the capture windows | On a 2× screen (a Retina Mac, most laptops) a picture shown at 720 CSS px needs 1440 physical px; a 1280 px file has 0.89 of what it needs, an 800 px file 0.56. The browser enlarges it |
| 2 | **Stretched on the landing page.** Each image is drawn at the full width of its column (`width: 100%`) with only one file offered and no `srcset`; the image's own width is only its layout hint | A 560 px palette image or an 800 px Tower image shown in a column wider than that is enlarged even on a 1× screen; the 1280 px hero image fills the 1280 px maximum width | The most visible blur, on the page the operator sees first |
| 3 | **No 2× file and no `srcset` on the docs.** No responsive image layout is set in the docs build configuration, so the built page offers one size; the site also has a click-to-zoom view that draws an image larger than its column | The build configuration; the zoom styles in the docs stylesheet | Fine on 1× screens, soft on 2× ones, softer again when zoomed |
| 4 | **Lossy and small web encodes.** Landing stills are WebP at quality 82, resized with a Lanczos filter to the manifest width; the clip is H.264 at CRF 26 and 800 px wide; GIFs use 192 colours | `publish-landing.mjs`, `recorder.mjs` | A smaller part of the softness, but it removes fine text edges (small type is the first thing to look wrong) |
| 5 | **Crops of a 1× window.** Many docs crops are small panels (a menu 330×381, a popover 366×390) of a window captured at 1× | The sizes in §4.1 | A 2× screen shows a 330 px menu at 330 px worth of pixels in a 660 px slot |

Not a cause: the pipeline does not upscale anywhere (every resize is a reduction). The blur is in the capture and in how the pages show the files.

**What was not verified:** the real rendered width of each image on each page. The spec's slot widths (§4.3) are the layout's limits, not measurements; the check script (§5.3) measures the rendered size in a real page at scale 2, which settles them, and the first run of that script replaces the estimates below.

### 4.3 The table (file, pixels, shown at, ratio)

"Slot" is the CSS width the image can be shown at (docs: the content column, about 720 px at the Starlight default; landing: the column the image sits in, up to 1280 px). "Needed" is 2 × slot. "Ratio" is pixels ÷ needed (1.0 or more passes).

| Group | Pixels | Slot (CSS px) | Needed | Ratio | Passes |
|---|---:|---:|---:|---:|---|
| Docs widgets, medium | 1280 | 720 | 1440 | 0.89 | no |
| Docs widgets, small | 800 | 720 | 1440 | 0.56 | no |
| Docs settings | 1280 or 1200 | 720 | 1440 | 0.83 to 0.89 | no |
| Docs memory | 1100 | 720 | 1440 | 0.76 | no |
| Docs agent | 960 | 720 | 1440 | 0.67 | no |
| Docs chrome crops | 128 to 560 | the crop's natural width up to 720 | 2× its slot | at most 0.5 | no (a crop is shown at its own width or stretched) |
| Landing hero and `workspace`, `ui-swarm` | 1280 | up to 1280 | 2560 | 0.5 | no |
| Landing `ui-memory`, `ui-connectors` | 1100 | about 600 to 1100 | 1200 to 2200 | 0.5 to 0.9 | no |
| Landing `ui-working`, `ui-question`, `ui-picker` | 960 | about 600 to 960 | 1200 to 1920 | 0.5 to 0.8 | no |
| Landing `ui-tower` | 800 | about 600 to 800 | 1200 to 1600 | 0.5 to 0.67 | no |
| Landing `ui-palette` | 560 | up to the column | 2× slot | under 0.5 | no |
| Landing clip | 800 wide | the column | 2× slot | under 0.5 | no |

The full per-file list (104 + 11 rows, with the ratio per file) is produced by the check script's first run and attached to the build's first PR. Exempt from the 2× rule: SVG, the 1200×630 link-preview images (a fixed size others define), the favicon and touch icons, and store badges.

---

## 5. The capture and export standard, and the check (C, continued)

### 5.1 The standard

| Rule | Value |
|---|---|
| Capture scale | Device scale factor **2** for every still and every clip, set by the capture's own viewport override (so it does not depend on the screen the host has). 3 only for a shot whose slot is wider than 1280 CSS px |
| What is captured | The real app window's page, as the tooling does now: no OS screenshot, no upscaled or screen-grabbed bitmap |
| Master | A lossless PNG at the captured size (for example 2560×1600), kept with the manifest entry. Never edited by hand and never recompressed |
| Web stills | Two files per image from the master: a 1× (slot width) and a 2× (twice the slot width), WebP at quality 90 or higher (lossless where it is smaller), listed with `srcset` and `sizes`. The 2× file is never smaller than 2× the slot |
| Docs | The 2× file is the one committed; the docs build derives 1× and `srcset` from it (decision 6 says how). The docs get a responsive image layout so the built page carries the `srcset` |
| Landing | The page offers 1× and 2× with `srcset`, and stops drawing an image wider than its 1× width |
| Clips | Captured at a 1280×720 viewport at scale 2 (2560×1440 frames; 1920×1080 viewport at scale 2 for a hero clip, 3840×2160 frames), encoded to H.264 at **1080p minimum** with CRF 20 or lower, 30 fps, plus a WebP poster from the master frame. 4K output is not produced (decision 1). No frame is ever scaled up |
| Titles, crops and zooms in a clip | Cropped and zoomed from the 2× frames, never from a 1080p render |
| Colour and edges | No resampling filter on a still except the one reduction from master to 1×; no sharpening |

### 5.2 Which host captures which, and how it is proven

The scale comes from the capture's own override, which works the same on every OS, so any host can capture a 2× image. What differs is the window chrome in a full-window shot and the fonts.

| Host | Role | Notes |
|---|---|---|
| The macOS testbed | Series clips (as Maop's pilot) and macOS-looking stills | Retina is natively 2×, but the override is still set so the result does not depend on the display |
| The Windows capture host | Landing and docs stills (the primary download is Windows); the hero window shot | Display scaling is ignored: the override fixes the scale. Docker is needed only for W4 |
| The Linux testbed | Nothing by default; a spot check that a still taken there has the same pixel size | Its fonts differ, so it is not a source of shipped images |

**Proof, for every image:** the manifest entry records `capture_scale`, the CSS size and the output size; the check (§5.3) fails an entry whose output is not exactly CSS size × `capture_scale`, and a calibration shot (a page of one-pixel lines and small text) is captured once per host per release and checked by eye and by the script.

### 5.3 The check script

`scripts/ui-screenshots/check-crisp.mjs` (built by the build task) fails, with a line per file, when:

1. an entry's `capture_scale` is below 2, or missing;
2. an image's width is under 2 × the width of its slot (the slot is read from the manifest, then confirmed by loading the built page at scale 2 and comparing each `img`'s natural width with its rendered width × 2; `currentSrc` must be the 2× file);
3. a built page has an image with no `srcset`, or a `srcset` whose largest file is under 2× the rendered width;
4. a clip's height is under 1080 or its source frames were below 2× its slot;
5. an image or clip has no manifest entry (so it has no recorded scale).

It runs in the refresh command before publishing, on the docs and landing builds, and as a per-release job. Its first run produces the list of current failures: today that is every one of the 104 docs rasters and all 11 landing files (§4.3).

---

## 6. One pipeline from the seed to the files (D)

```
seed profile  ->  capture / record at scale 2  ->  redact + verify  ->  masters  ->  derive web files  ->  check-crisp  ->  publish
```

| Step | What | Reuses |
|---|---|---|
| Seed | `dev-seed` profiles from #30225 play the story in an isolated home on a release build; the same profile feeds stills and clips, so the Swarm row in a still is the row in the clip | #30225 |
| Capture | Stills by suite, clips by scene, both at scale 2; a scene is a script in the repo | `capture.mjs`, `record.mjs`, Maop's recorder |
| Timestamps | The seed plays on srv's clock at capture time, so every time on screen is "now"; a clip is captured in real time; a still is taken after the story has played to its frame | #30225 |
| Nothing real | Made-up names and repos, a fake secret, the made-up account, no host names or user paths; the capture home is isolated; redaction rewrites home and host names live | `redact.mjs`, `demo-env.mjs` |
| Verify | Page-text check after every beat, `containsWorkspaceData` gate on stills, the OCR frame scan on every clip against a blocklist built from the capture machine, and a person reads every image and watches every cut | The series' four layers |
| Masters | PNG masters and 2×-frame sources kept (§7) | |
| Derive | 1× and 2× WebP, the poster, the 1080p MP4 | `publish-landing.mjs`, extended |
| Check | `check-crisp.mjs` (§5.3) | New |
| Publish | `publish.mjs --docs`, `--landing`, and the media repository; refuses an image that failed a check or was not marked reviewed | `publish.mjs` |
| **Retaking when the UI changes** | `node scripts/ui-screenshots/refresh.mjs --only <id>` retakes one image or clip from the seed; with no `--only`, the whole set. Each manifest entry carries the AgentMux version and date, and a per-release job lists images older than a chosen number of releases | `refresh.mjs`, coverage report §5.9 |

Two things the build reconciles (both named in the series spec): the seed's placeholder account address against the screenshots' demo address, and the seed's refusal of a non-dev build against filming on a release build (an opt-in flag).

---

## 7. Where the files live (E)

Public repositories get finished material only: images and clips the operator has seen.

| What | Where | Size |
|---|---|---|
| Docs images | The docs repository, `src/assets/screenshots/<suite>/`, the 2× WebP only (the build derives the rest) | About 125 images × 100 to 250 KB, about 15 to 30 MB; today 5.6 MB |
| Landing images and clips | The landing repository, 1× and 2× WebP, the 1080p MP4 and poster | About 2 MB per image pair, 10 to 20 MB per clip |
| Masters (lossless PNG, 2× sources) | The media repository, in Git LFS, per set: the first release set (5 stills and 1 clip) and the series stills; the rest is retaken from the seed, not stored | A 2560×1600 UI PNG is roughly 0.4 to 0.9 MB; the first release set is under 50 MB |
| Rendered video | The media repository (LFS) and a release attachment per approved episode | As the series spec |
| Scripts, scenes, storyboards, manifests | The agentmux repository (`scripts/ui-screenshots/`, `docs/`), as text | Small |

Nothing private goes in a public place: no host names, user paths, account names, keys, or details of the private repositories. The publish guard and the media repository's push guard apply.

---

## 8. Voice and captions (F)

| Item | Decision | Notes |
|---|---|---|
| Series voice | Already settled (series spec): a local open-licence voice, speed 1.0, no cloned voice | Free; a person listens to every cut |
| Landing clips | Silent loops with a pause control, as today | Each has a text description; no narration to review |
| Captions | A caption file from the same timeline as the narration, delivered with each series video; the landing clips carry none (they are silent) | Burned-in captions only if the operator wants them: the pipeline host has the fonts, the GPU host does not |
| The video-model host | Used for background or extension work at most. It has no speech or caption tool, no 4K, and it cannot draw a readable interface, so every screenshot and every workflow clip is captured from the real app | Measured: about 3.3 hours of GPU per minute of 720p video, one job at a time; not needed for this plan |
| Claims | Every sentence that says what the product does is checked against the shipped build by the "Confirm" line in the scene's script before recording (series spec §3); a claim that is narrower than the line is changed to what is true | The Vault episode's "never reads" is the one most in need of the Vault owner's review |
| Disclosure | The description of each series video says an AI voice and demo data were used | As the series spec |

**Options for the voice** (the operator's decision 4, with the series spec's answer as the default):

| Option | Cost | Time | Note |
|---|---|---|---|
| A. The local voice already chosen | $0 | Part of each episode | Recommended; settled in the series spec |
| B. A person records | The person's time | Per re-take: a re-record when the UI changes | Better warmth, but every UI change needs a new recording |
| C. A commercial text-to-speech service | A per-minute fee and a vendor account | Quick | Needs the operator's OK for the account and spend; no benefit over A |

---

## 9. Order and cost (G)

| # | Step | Result | Effort | Who |
|---|---|---|---|---|
| 0 | The operator's decisions (§10) | Standard, first-five list, hosts, order | | The operator, through AgentY |
| 1 | Crisp standard and `check-crisp.mjs`; scale 2 default in `sizes.mjs` and the recorder; `srcset` in the docs build and the landing page; the failing list | The standard is enforced; the list of what fails | 1 to 1.5 days | Loap (tooling); the docs and landing owners for the two sites |
| 2 | The seed's `landing` and `kanban-story` profiles (#30225 build) | A busy, resettable story | 2 to 3 days | Loap |
| 3 | **Release 1: 5 landing stills (W1 to W5) and 1 clip (W1)**, captured at 2×, checked, swapped into the landing page | The first crisp, workflow-led set | 1 to 1.5 days after step 2 | Loap (scene and capture), the Windows capture host's agent (build and capture), the landing owner (swap) |
| 4 | **Stale docs pictures**, high-confidence items from the audit first (the pane header, the Settings tab strip, the picker text, the removed Media and Warden panes, the status bar, menus), then medium; the unreferenced files and the removed-feature files are deleted by the docs fix | 55 or so retaken images, in batches by page group | 3 to 4 days, in 4 to 6 PRs | Loap (capture), the docs owner (pages) |
| 5 | **The series**, E01, E03, E04 first, then the rest (series spec decisions 1 to 3) | Episodes in Maop's order | Per the series spec: about 2 days for the first three, then 2 to 3 hours each | Maop |
| 6 | Light-theme set, recordings for the docs, per-release refresh job | Variants and freshness | 2 days | Loap |

Real-model episodes (E07 to E09) cost a few cents to a few dollars of tokens each. Everything else costs agent time only. Captures run on the testbed hosts and the Windows capture host; a capture is a scripted UI run, so the host's custodian is told first and a run on the operator's own machine needs the operator's OK (none is planned).

---

## 10. Decisions for the operator (H)

1. **Approve the standard** (§5.1): capture at scale 2, lossless masters, 1× and 2× web files with `srcset`, video at 1080p from 2× frames, and the check that fails anything under 2× its slot. *Recommended: yes. No 4K output: a 4K master costs four times the storage and the sites cannot show it.*
2. **The workflow list** (§3): confirm, strike or add rows, and name the first five. *Recommended: W1 to W5 for the landing page, then E01 to E09 for the series, with W6 and W7 for the docs.*
3. **Which host takes the stills.** *Recommended: the Windows capture host for landing and docs stills (the primary download), pane crops without OS chrome everywhere except one hero window shot, and the macOS testbed for the series clips as proposed in the series spec.*
4. **The voice.** *Recommended: keep the local voice already chosen (option A); silent clips on the landing page.*
5. **The order** (§9). *Recommended: standard and check, then the seed, then release 1, then the docs retake, then the series; the series' first three episodes can start once the seed's `three-agents` profile exists.*
6. **Docs image source.** *Recommended: commit only the 2× WebP to the docs and let the build derive 1× and `srcset` (about 15 to 30 MB in the repository), with masters in the media repository for the first release set only; the alternative is to commit 2× PNG masters to the docs (about 75 MB of history that never shrinks).*
7. **Scope of the docs retake.** *Recommended: the high-confidence stale images first (the audit's "high" rows), then medium, and delete the unreferenced and removed-feature images; the low-confidence and "cannot tell" ones are retaken when their section is next touched.*
8. **A cap on churn.** *Recommended: retake once per release, not per PR; the per-release job lists images older than two releases.*

Labs: every capture and recording this plan describes runs on a testbed host or the Windows capture host and is told to that host's custodian first. Nothing runs on the operator's own machine without a separate OK, and approving this spec is not that OK for the series' takes (the series spec asks for its own).

---

## 11. Risks

1. **Private material on a frame.** Four layers (live redaction, page-text check, OCR scan, a person) as the series; one more for stills: the `containsWorkspaceData` gate. Native views (a browser pane's page) are not in a page capture; those shots use the compositing the coverage report plans, or stay text.
2. **A scene claims what the product does not.** Each scene's script has a "Confirm" line checked on the release build before recording.
3. **File size.** 2× images are about four times the pixels; WebP and the 1×/2× split keep a page's download close to today's for a 1× screen.
4. **Churn.** The UI changes weekly in alpha; a per-release retake and the version stamp keep the pictures honest without a weekly cost.
5. **The seed is the critical path.** Steps 3 to 5 wait for it. If it slips, the crisp standard and the retake of the stale docs pictures that need no content (menus, Settings, chrome) can still proceed.
6. **The slot widths are estimates until the check measures them.** The first run of the check replaces §4.3's estimates with measurements; the standard does not change.

---

## 12. What approving this spec means

The operator approves in conversation, the spec's task is completed, and the build (#48241) starts: step 1 first (the standard, the check, the two sites' image handling), then the seed, release 1, the docs retake and the series, each as its own PR with its own review. The audit's text fixes are not part of this build; the docs owner takes them from the audit directly.

Agent: Loap@narko

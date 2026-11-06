# SPEC: Practice time for building computer-use skills, and two new ones: Voicemeeter and Cantabile

**Status:** draft — awaiting operator approval; task #1627 (build #1628 starts after it).
**Date:** 2026-11-05
**Author:** Manoz@Area54, at AgentY's request after the operator asked (quoted): "lets also add voicemeeter and cantabile as apps for our computer use. Curious: do agents need time working with the app to build the pack, would that help?"
**Written against:** origin/main `c655acddd` (v0.60.17); Stage 1 PR [#6549](https://github.com/agentmuxai/agentmux/pull/6549).
**Addendum to:** `SPEC_PACKS_2026_11_05.md` (format §3, trust §5, proof §7, versions §10, stages §14).
**Naming:** the operator approved (quoted via AgentY, 2026-11-05) "computer use skill", with the badge "App skill", never "plugin", living in Memory > Skills with an "App" filter and no separate Packs tab. This spec uses those words. `SPEC_PACKS_2026_11_05.md` and the Stage 1 files still say "pack"; renaming them is a follow-up (§9).

## Decisions for the operator

| # | Decision | My recommendation |
|---|---|---|
| V1 | Does practice time help? | **Yes, if it is bounded, evidence-tagged, reviewed and measured.** Unreviewed self-written notes can make an agent worse (§1). |
| V2 | Which machine do practice and tests run on? | A **dedicated test machine or a VM** for Voicemeeter (its audio driver is machine-wide, so a test *user* is not enough), and a test machine or test user for Cantabile. I need you to name it (§6). |
| V3 | Session size | **90 minutes of wall-clock and 150 `Desktop` actions** per app per session, one session first (§3). |
| V4 | Cantabile edition for the test machine | **Performer on its 30-day trial.** The network API is Performer only; Lite is free but cannot be driven (§5). Extending the trial is a request to Topten support. |
| V5 | Who runs the first sessions | A Windows agent with `Desktop` on, on the test machine; today that waits for you to switch `desktop:enabled` on. Voicemeeter's and Cantabile's API routes can be tried without it (§7). |

## 1. Does practice time help? What the evidence says

The operator's question has a real answer, and it is conditional.

**For:** [AppAgent](https://arxiv.org/abs/2312.13771) (CHI 2025; v3 revised July 2026) lets a model explore an unfamiliar app by trial and error and writes a document of what each element does. On 45 tasks over 9 Android apps with GPT-4, the success rate was **48.9% with no document, 73.3% with the exploration-built document, 84.4% from watching demonstrations and 95.6% with a hand-written document**. Exploration capped at 40 steps and deployment at 10. So practice helps, but a person's knowledge helps more.

**Against, and why this spec has the shape it has:** [SkillsBench](https://arxiv.org/abs/2602.12670) (87 tasks, 18 configurations, Feb 2026) found that **curated** skills raised the average pass rate from 33.9% to 50.5%, while skills an agent wrote **for itself** before solving a task lowered it (for three configurations, 8 to 12 points below having no skills; I read these figures in a summary of the later sections, so check Appendix D.6 before quoting them). The causes it reports: the agent did not open its own notes, the writing displaced the real work, and confident wrong statements were locked in. Short skills beat long ones.

Neither study tested what we propose, and AppAgent is a 2023 result on phones. So the design borrows the good part and guards the bad:

1. Practice is a **separate, bounded session** with a task list, not an agent writing a skill on its own initiative.
2. Findings go to a **practice log that is not loaded into any agent's context**. Only distilled facts reach the skill, as **proposed edits a person reviews** (the "learned in use" rule of SPEC_PACKS §10.3).
3. A fact enters only with evidence: seen twice, or confirmed by a test step, with the app version.
4. **The measure is a matched cold run** (§4): the same tasks with and without the skill, in a fresh session. If the skill does not raise the pass rate or cut the steps, it is not shipped as improved. This is SkillsBench's own design applied to one app.
5. The skill stays **compact** (SPEC_PACKS §3: a router `SKILL.md` under about 60 lines, reference files read on demand).

## 2. Practice time as a step of building any computer-use skill

It joins the Write-a-skill guide as a step after "read the vendor's documentation" and before "write the test".

| Step | What | Output |
|---|---|---|
| 1. Task list | Written **before** the session from the vendor documentation and what real users do: 10 to 15 jobs, each with the route to try first (API, then plug-in, then keys, then `Desktop`) and a verify cue a machine can check (a file, a value read back, a window title) | `practice/tasks.md` |
| 2. Set-up | The test machine, the app version and edition recorded, a snapshot or restore point, the per-app `Desktop` grant asked at that moment, Stop in reach (§6) | session header |
| 3. Session | The agent works the tasks in order, API route first. It logs every task: the route tried, what worked, what did not, any dialog or pop-up that appeared, and a screenshot hash for each step that mattered. A failed route is as valuable as a working one | `practice/log-<n>.md` |
| 4. Distil | A **fresh** agent (not the one that ran the session) reads the log and drafts proposed edits to the skill's files, each fact tagged `seen on <app version>` and pointing at a log line. It drops what was seen once and unconfirmed | a diff |
| 5. Review | A person reads the diff. Nothing enters the skill without it | an approved diff |
| 6. Tests | The tasks that passed and have a machine-checkable cue become `test.json` | the skill's test |
| 7. Cold run | Step 4 in §4 | pass records |

**Where the files live.** `practice/` is kept locally in the author's workspace and not shipped in the skill or the public repo (`SPEC_SKILLS_REPOSITORY`): logs can hold screenshots of a person's screen and file paths. A shipped skill carries only the distilled, tagged facts and the test.

## 3. How long a session is, and what it costs

- **Bound: 90 minutes of wall-clock and 150 `Desktop` actions per app, whichever comes first.** `Desktop` is limited to 2 inputs a second already; the action cap is what keeps a session from wandering. API-route work (scripts, the Remote API, the network server) does not count against the action cap, and is the first thing tried.
- **One session first, a second only if the cold run (§4) shows the skill is thin.** More practice has diminishing returns in AppAgent's setup (exploration stopped when the task was done); I would stop at two sessions per app version.
- **Cost (my estimate, not measured).** Each `Desktop` look is one screenshot scaled to about 1,568 pixels on the long edge. 150 look-act-verify steps with reasoning is of the order of a large build task on a model with caching. I cannot give a dollar figure; the first session records the action count, wall-clock and tokens in its log header, and I will report the measured figure with the Voicemeeter result. I will not quote a number before that.
- **Cost of not doing it:** the skill is written from documentation alone, which is what the Stage 1 Word, Excel, Ableton and WaveLab skills are (marked unverified). Practice is what turns "unverified" into a pass record.

## 4. The cold run: how we know it helped

After step 6, run the practice tasks again in a **new session with no practice log**, twice:

- **A: no skill** (the agent has only the `Desktop` and MCP tools);
- **B: with the skill.**

Record per task: pass or fail, `Desktop` actions, wall-clock. The skill is "verified" (SPEC_PACKS §7, §10.2) only if B passes the test task. The A/B difference is stored with the pass record (`baseline: { passRate, actions }`) and shown in the skill's detail view as "adds N tasks passed, saves M actions". If B is no better than A, the skill is rewritten shorter, not longer. Small samples: this is 10 to 15 tasks, so it tells us "clearly worse", "about the same" or "clearly better", not a percentage to the decimal.

## 5. The two apps

Everything below is from vendor pages read on 2026-11-05. What I could not read is marked, and each such item becomes a practice task rather than a claim.

### 5.1 Voicemeeter (VB-Audio; Standard, Banana, Potato)

- **Versions:** Standard 1.1.3.0, Banana 2.1.3.0, Potato 3.1.3.0 (packages dated Sept 2026); the Banana package installs Standard and Banana, the Potato package all three. The Remote API reports type 1, 2 or 3.
- **Rung 1, the Remote API** (the SDK is on GitHub as [vburel2018/Voicemeeter-SDK](https://github.com/vburel2018/Voicemeeter-SDK)): "standard Windows DLL installed with Voicemeeter", one 32-bit and one 64-bit. From `VoicemeeterRemote.h`: `VBVMR_Login` (0 ok, 1 ok but Voicemeeter not launched), `VBVMR_RunVoicemeeter(type)`, `VBVMR_GetVoicemeeterType`, `VBVMR_GetVoicemeeterVersion`, `VBVMR_IsParametersDirty` (poll about every 10 to 20 ms, one thread), `VBVMR_GetParameterFloat/Set…`, and `VBVMR_SetParameters(script)` for up to 48 kB of lines separated by `,` `;` or newline (a positive return is the failing line number). Parameter names look like `Strip[0].Gain`, `Strip[0].Mute`, `Bus[0].Gain`, `Strip[0].A1 = 0`. **Four client applications** can be connected at once. The header says use is "driven by a license agreement" in `VoicemeeterRemoteAPI.pdf`, which I did not open; the README says the SDK's code is free in any project that uses the DLL or VBAN. **The skill must read that agreement before it ships a helper script.**
- **Wrappers:** [`voicemeeter-api`](https://github.com/onyx-and-iris/voicemeeter-api-python) (MIT, `pip install voicemeeter-api`, Python 3.10 or later, needs Voicemeeter installed, tested on 1.1.2.2, 2.1.2.2 and 3.1.2.2; last activity not seen) and [`voicemeeter-remote-python`](https://github.com/chvolkmann/voicemeeter-remote-python) (MIT, README tested on a Sept 2019 Potato; older). The skill pins a wrapper version and carries it as a `scripts/` helper under the approval rule (SPEC_PACKS §5), or calls the DLL through `ctypes` with no dependency.
- **Rung 2, VBAN-TEXT** (UDP; Voicemeeter lists "1x VBAN-TEXT Input stream") and **Macro Buttons** (up to 80 buttons; triggers from keyboard, MIDI, HID, game pad, VBAN-TEXT or an audio level; actions that send instructions to Voicemeeter, MIDI, a VBAN-TEXT script, keys or run a program; settings in `My Documents\Voicemeeter`; version 1.1.4.1). **Not read:** VBAN-TEXT's port, syntax and authentication, and the Macro Buttons instruction list, which are in the VBAN specification PDF and the Potato manual (the first would not parse and the second was over the fetch size limit). The first practice task is to read both.
- **Rung 3, shortcuts:** nothing found; a practice task.
- **Rung 4, `Desktop`:** Voicemeeter draws its own UI. **UI Automation:** I found no statement either way. Practice task zero is to dump the UI Automation tree of its main window (once `Desktop` has `tree`, SPEC_PACKS §12) or with Windows' own inspector, and record what it sees, before any rung 4 instruction is written.
- **Install:** "Run Setup program in administrative mode / Reboot after installation (and Reboot after de-installation)". It adds a virtual audio device and driver. Donationware, a licence prompt after 30 days, no account.
- **Test (rung 1):** log in, read `Strip[0].Gain`, set it to −6.0, read it back, set `Bus[0].Mute`, read it back, restore both. Pass if each read-back equals what was set. This needs no audio to flow and no `Desktop`.
- **Traps to write down after practice, not before:** the poll-and-dirty-flag pattern, 32 versus 64-bit DLL, the engine restart that drops audio, and what happens when a client logs out.

### 5.2 Cantabile (Topten Software)

- **Versions:** the 4300 series (build 4360, Aug 2026, the newest not marked experimental) and the 4400 series (July 2026, **all experimental, "definitely not be used for live performance"**; .NET 10). 4300 and later need Windows 10 Anniversary or later, 64-bit. The skill targets the newest non-experimental build and says so.
- **Editions:** Lite (free), Solo (US$69), Performer (US$199). **The network server, the web UI, the JavaScript library and the Stream Deck plug-in are Performer only.** Bindings are Solo and Performer. A 30-day fully functional trial of Solo and Performer is in the same installer; what happens after 30 days is not stated on the pages I read.
- **Rung 1, the Network Server** ("Cantabile Performer Only"): "minimal control… using a REST API and full control via a WebSocket interface". **Off by default**; enabled in Options (the guide says Options → Miscellaneous; the release notes say Tools → Options → Misc → Network Server). **Port 35007. Not password protected or authenticated**: "the only way to restrict access… is via Windows Firewall and/or by isolating to a local network". The installer creates the firewall rules. Confirmed endpoint: `http://localhost:35007/api/bindings/availableBindingPoints`. The protocol reference itself (`/netapi`) is rendered by JavaScript and I could not read it; I have no full endpoint list or message schema.
- **Rung 1 client:** the official [`cantabile-js`](https://www.cantabilesoftware.com/jsapi) (v0.3, "not backwards compatible with previous versions", Node and browser; v0.3.4 released; **licence and last commit not verified**). It exposes `song`, `setList`, `bindings`, `commands`, `variables`, `transport` and others, and raw `send/request` over the socket. A value is set with `bindings.invoke(bindingPoint, value)` (for example `bindableId: "masterLevels", bindingPointId: "outputGain", value 0.5`), read with `bindings.query`, and watched with `bindings.watch`. I found no "list plug-ins" call; plug-ins are reached by index (`indexedPlugin` with `rackIndex` and `pluginIndex`).
- **Rung 2, bindings and MIDI:** a binding joins a source point to a target point through a mapper; the same points are reachable from the network API. No OSC support was described on the pages I read.
- **Other:** command line (`/headless`, `/livemode`, `/state:`, `/config:`, `/minimized`, a song or set-list file), and an official **Stream Deck** plug-in that "invokes any binding target". No embedded scripting language found. **UI Automation:** no statement found; practice task zero, as for Voicemeeter.
- **Licensing on a test machine:** licences are kept in `AppData\Local\Topten Software\Cantabile3.licenses` and tied to a hash of the Windows product key; offline activation exists.
- **Test (rung 1):** enable the server, fetch `availableBindingPoints`, `invoke` master output gain to 0.5 and `query` it back, restore it. Pass if the read-back equals 0.5.
- **Security the skill must say:** the server has no authentication and listens beyond localhost by default through the firewall rules the installer adds. On the test machine, restrict it to localhost (or the LAN you control) and turn it off when the session ends. A skill must never tell an agent to enable it on a machine with a live set.

## 6. Safety: where practice and tests may run

The operator's rule is quoted in the task: Voicemeeter routes the whole machine's audio and Cantabile hosts live plug-ins.

- **Never on the operator's live audio setup.** Practice and tests run on a **dedicated test machine or a VM**, named by the operator (V2) before the first session. A guard in the task: the agent reads the machine's name from the session header and the operator's confirmation, and does not start otherwise.
- **Voicemeeter needs a machine, not a user.** Its driver and device routing are machine-wide, installed with admin rights and a reboot, so a separate Windows user on a shared machine would still change everyone's audio. Use a VM or a spare machine. Cantabile alone is safer on a test *user* if no real audio device is selected.
- **No sound path to a person.** Configure the test machine's outputs to a virtual or null device, no microphone, no speakers; practice tasks cap gain at 0 dB and never raise a master output above a quiet level. A plug-in under test is a free or demo one chosen by the person.
- **`Desktop` control stays gated** (SPEC_DESKTOP_DRIVING): the `desktop:enabled` switch on that machine, a grant per app at the time, browsers and password managers refused, Stop, the audit log. **On Area54 the switch is the operator's and is still off**; nothing here turns it on.
- **A snapshot or restore point before each session,** and the session ends by restoring the app's settings (Voicemeeter's saved configuration, Cantabile's `/config:` and `/state:`) to what the header recorded.
- **No account, no payment.** A trial extension is a request to the vendor, made by a person.
- **Practice logs stay local** (§2); screenshots can show a screen.

## 7. What can be done before `Desktop` is on

Most of both apps' value is in rung 1 and needs no `Desktop`:
- Install both on the test machine, read the documents listed as unread (VBAN specification, Potato manual, `VoicemeeterRemoteAPI.pdf` licence, Cantabile `/netapi` once rendered in a browser), run the two rung 1 tests, and write the skills' rung 1 and 2 sections with the Stage 1 `verified` records. Rung 3 and 4 sections and the cold run wait for `Desktop`.
- A practice session limited to the API routes is allowed first and costs little.

## 8. What changes in the other specs

- **`SPEC_PACKS_2026_11_05.md`:** add a §4a "Practice" (this spec's §2 to §4) and the `practice/` folder rule; add `baseline` to `verified` records (§4); the table in §11 gets two rows (§5 here).
- **`write-a-pack`** (to be renamed): a new step "Practice" between "read the documentation" and "write the test", with the task-list template.
- **`test.json`:** a field `source: practice | documentation`, so a reader can tell tested-from-use from tested-from-text.
- **Stage 1 skills:** Word, Excel, Ableton Live and WaveLab get a practice session when the first test machine with their app is named. The skill's card says "from documentation" until then, which it already does.

## 9. The naming follow-up (AgentY's request)

[#6549](https://github.com/agentmuxai/agentmux/pull/6549) is docs and example files only; it adds **no UI and no Packs tab**, so there is no UI code to move. What it does contain is "pack" wording and a separate "Packs tab" in `SPEC_PACKS_2026_11_05.md` (§8 and its mockup), the `docs/examples/packs/` folder, and the guide. I will open one follow-up PR **after #6549 merges** that renames to "computer-use skill" (badge "App skill"), moves the Memory section to Skills with an "App" filter, and renames the examples folder; no code. This is a docs-only change to files that have already merged, so I will say so in the PR rather than hide it. `pack.json` and `.pack` stay as file names unless you want them renamed too.

## 10. Stages

| Stage | What | Cost |
|---|---|---|
| 0 | Name the test machine (V2); install both apps; read the unread documents | half a day, mostly waiting on the machine |
| 1 | Rung 1 and 2 sections and tests for both skills; the practice section in the guide; the two rung 1 tests run | 2 to 3 days |
| 2 | A practice session per app with `Desktop` (needs the switch); distil, review, cold run | 2 days plus review, with one measured cost figure |
| 3 | The `practice/` and `baseline` fields in the format; the skill card shows "adds N tasks passed" | with the Packs build (#1560 Stage 2 and 3) |

## 11. Open questions

1. Which machine is the test machine, and is a VM acceptable (V2)?
2. Does the Voicemeeter Remote API run with no audio device present? I did not find a statement; the rung 1 test is how we learn.
3. Is `cantabile-js`'s licence compatible with shipping a copy in a skill? Not verified; the safe route is to cite it and call the REST API with plain HTTP.
4. Do VB-Audio's terms allow an automated agent to control the app through the Remote API? The README says the SDK is free in any project that interacts through the DLL; the PDF's licence agreement is unread.
5. Whether practice time should be a documented, recorded step in every skill's changelog ("practice session 1, 142 actions"). I recommend yes: it is the evidence for "from use".

Agent: Manoz@Area54

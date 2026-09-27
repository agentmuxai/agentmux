# SPEC: `muxlog admission` — one cross-instance timeline for "why won't this agent run here"

**Date:** 2026-09-27
**Status:** implemented — #3919 (spec #3916; see §7 for where the implementation differs from the first draft)
**Author:** Manoz
**Repos touched:** `agentmux` (`agentmux-srv/src/backend/shellintegration/muxlog.mjs`, `docs/MUXLOG.md`)
**Related:** `docs/MUXLOG.md`; `SPEC_MUXLOG_SWARM_DISPATCH_VERDICT_2026_08_22.md` (same "recipe + verdict" pattern); the one-live-instance-per-agent work and its take-over fixes (#3897, #3899, #3903, #3908)

## 1. Problem

On 2026-09-27 AgentA was stuck, and its pane's **Take over** button did nothing. Two AgentMux instances were running on the same computer (0.57.6 and 0.57.8). Answering "why" meant reading **both** instances' sidecar logs **side by side**, across **two** daily log files each, and matching the agent by name *and* uid. muxlog couldn't do any of those four things, so the root cause took a manual file hunt plus hand-written `grep`/`awk`.

The root cause, for reference (all from srv logs):

| UTC | Instance | Event |
|---|---|---|
| 23:52:19 | 0.57.6 | AgentA `self-quit`; `registry: entry changed hands … skipping remove`. So the cloud subscription is **not** dropped, and the relay lease keeps being renewed every 5 s (#3897's bug). |
| 23:53:11 | 0.57.8 | `agent_admission.granted` (epoch 6) and CLI spawned |
| 23:56:31 | 0.57.8 | `agent_admission.fenced`: the relay says 0.57.6 holds AgentA, so the CLI is stopped |
| 23:57:57 | 0.57.8 | Take over: `requesting release from the holder` |
| 23:57:57 | 0.57.6 | `takeover: release handled`, **`released: 0`**. There's no local block, so nothing is released, and the relay lease isn't touched (#3899/#3908's bug) |
| 23:57:57 | 0.57.8 | `granted` (epoch 7) and CLI spawned |
| 23:58:09 | 0.57.8 | `fenced` again: the relay still names 0.57.6 |
| 23:58:17 | 0.57.8 | `denied`, and every later attempt is denied too |

That table is exactly what one command should print.

## 2. Gaps found (muxlog on main at 7042bb4, identical to the deployed `~/.agentmux/shell/muxlog.mjs`)

- **G1 — only the newest rotated file.** Sidecar and host logs roll daily (`agentmuxsrv-v<ver>.log.<YYYY-MM-DD>`, by UTC date). muxlog resolves one file per target: the newest. An incident just before UTC midnight is invisible, even with `--since 2026-09-26T23:50`. `muxlog srv -i 6addbd3a --since 2026-09-26T23:50 --grep admission cat` returned only the next day's unrelated lines.
- **G2 — one instance at a time.** `-i` picks a single instance. Take-over involves a **requester** and a **holder** in different channels, and their lines only make sense interleaved. The only merged view today is `phases`, which merges host and srv for one pane in one instance.
- **G3 — no agent filter over structured fields.** The agent appears as `fields.agent`, `fields.agent_id`, `fields.uid` (the definition id), or inside `fields.holder`. Case varies too: `AgentA` in some lines, `agenta` in others. `--grep` matches the message only, so filtering by agent needs a raw-line regex that hard-codes both spellings and the uid.
- **G4 — no recipe for the admission vocabulary.** The events live under several targets and messages: `agent_admission.*`, `registry: … changed hands`, `self-quit`, `persistent process spawned`, `muxbus: auto-registered…` and lease/subscription lines. There's no recipe like `auth`/`swarm` that knows this set.
- **G5 — the rendered line drops the facts that matter.** `holder`, `released`, `epoch`, `why`, `to` and `holder_channel` only appear with `--verbose`, and then with every other field too.

## 3. Design

### 3.1 Read across rotated files (fixes G1; applies to every target)

- For a resolved target, collect **all** rotated siblings (`<stem>.log.<date>`, plus the un-suffixed file if present), sorted oldest to newest.
- With `--since` (or the new `--until`), read every file whose date range can overlap the window. A file named for date D can hold lines from D only, so skip files dated before the `--since` date.
- Without `--since`, `cat`/`tail` behave as today: newest file only. No behavior change for existing uses.
- `ls` is unchanged: it already lists every rotated file as its own row, so no `FILES` column is needed (see §7).

### 3.2 `--instances all | <substr>[,<substr>…]` (fixes G2)

- Fan out over every instance `ls` lists (`--instances all`, live and stopped alike; `--instances live` drops the ones whose liveness probe says `dead`), or over the ones matching the substrings (see §7).
- Merge lines chronologically with the same `ts` sort `phasesTimeline` already uses.
- Prefix each rendered line with a short **instance tag**, `v<version>/<channel hash last 8>` (e.g. `v0.57.6/7a8245ae`), so holder and requester are unambiguous. Right after it, the version shows at a glance whether a known fix applies.
- `-i` keeps its current single-instance meaning. `--instances` is the multi-instance form.

### 3.3 `--agent <name|uid>` (fixes G3)

- Match the **raw** NDJSON (message and fields), case-insensitively, against: `fields.agent`, `fields.agent_id`, `fields.uid`, `fields.definition_id` and `fields.holder`, plus the message text.
- Given a **name**, first scan the window for lines that pair that name with a `uid`, then also match those uids. Given a uid, do the reverse. Both lookups are internal; the user passes one identifier.
- Composes with every target and recipe.

### 3.4 `muxlog admission [<agent>]` recipe (fixes G4, G5)

- **Implies** `srv`, `--instances all`, `--since <24 h ago>` (overridable) and `--agent <agent>` (default: `$AGENTMUX_AGENT_ID`, so an agent can check itself).
- **Vocabulary** (message regex, raw-line match): `agent_admission\.`, `registry: (shared )?entry changed hands`, `self-quit`, `persistent process (spawned|exited)`, `turn_active flip \(process exited\)`, `muxbus: auto-registered`, `muxbus: .*(lease|subscription|RemoveAgent)`, `provisioned per-agent credential`. Pass `--grep` to override, as with `auth`.
- **Inline fields** (without `--verbose`): `holder`, `holder_channel`, `released`, `epoch`, `block_id` (first 8), `why` (first 120 characters), `to`.
- **Verdict footer**, one line per instance, then a diagnosis:
  - the agent's last admission state in each instance (`granted@epoch`, `fenced`, `denied`, or none);
  - who the relay last named as holder;
  - whether that holder has a **live block** for the agent: its last `persistent process spawned` after its last `self-quit`/exit;
  - a **diagnosis line** from a small rule table. The first rule covers this incident: *"relay names `<holder>` but `<holder>` has had no live block for the agent since `<ts>`, and a take-over there answered `released: 0`. That's a stale relay lease: a closed agent's subscription kept renewing (#3897), and the take-over couldn't reach an instance that held only the relay lease (#3899, #3908). #3897, #3899, #3903 and #3908 are all in v0.58.0. Recover by quitting or restarting the holder instance. The lease expires ≤ 15 s after renewals stop (`LEASE_TTL_MS`), then Take over again. If the requesting instance predates #3903, it may keep its cached refusal after that; restart it too."*
- The same fix set (#3897, #3899, #3903, #3908) is what the "Related" line cites. The rule table cites each by the failure mode it fixes.
- The rule table lives in `muxlog.mjs` next to the recipe, one entry per known failure mode, each citing its fix PR. Unknown patterns print the timeline with no diagnosis line.

### 3.5 Docs

`docs/MUXLOG.md`: document `--instances`, `--agent`, `--until`, the rotated-file behavior, and the `admission` recipe, with this incident as the worked example.

## 4. Acceptance

- `muxlog admission AgentA --since 2026-09-26T23:50`, run against fixture copies of the two instances' `…log.2026-09-26` and `…log.2026-09-27` files, prints the §1 table's rows in order, tagged `v0.57.6/7a8245ae` and `v0.57.8/6addbd3a`, followed by the stale-relay-lease diagnosis.
- The same command with `agenta` or `d76da857-56b7-402f-80a4-1ceb7ab1d457` in place of `AgentA` gives the same output.
- Every existing `muxlog` invocation without `--since`, `--instances` or `--agent` gives byte-identical output (a regression fixture for `srv cat`, `errors`, `swarm -d`, `phases`).

## 5. Tests (`muxlog.test.mjs`)

- Rotated-file selection: the window spans midnight UTC; files dated before `--since` are skipped; there's no `--since` → newest only.
- Multi-instance merge order with identical timestamps: stable, instance order as `ls`.
- `--agent` name→uid and uid→name expansion; case-insensitive name match.
- `admission` verdict: stale-lease rule fires on the §1 fixture; no diagnosis on a clean grant; no false stale-lease verdict when the holder does have a live block.

## 6. Out of scope

- Instances on **other computers**: their logs aren't readable locally. The verdict says "holder is on `<host>`; run `muxlog admission` there".
- Changing what the sidecar logs. Everything above uses lines that already exist.

## 7. As built (differences from the first draft)

- **`--instances all` includes stopped instances.** The draft defaulted to live ones only. In a take-over incident the holder has often just been quit, and its lines are the evidence, so `all` means every instance whose logs reach the window, and `live` restricts to the ones the liveness probe doesn't report `dead`. Instances with no file dated inside the window are skipped, and the header lists only the instances that contributed lines.
- **No `FILES` column in `ls`.** `ls` already lists each rotated file as its own row.
- **`--agent` also matches the agent's panes.** Lines such as `persistent process spawned` and `turn_active flip (process exited)` carry only a `block_id`, so the block ids seen on the agent's own lines are matched too. Without them, the verdict can't tell whether the holder still has a live pane.
- **Verified on the real incident logs:** `muxlog admission AgentA --since 2026-09-26T23:50 --until 2026-09-27T00:00` prints the §1 timeline from both instances and the stale-relay-lease diagnosis. `agenta` and the uid give identical output. Existing commands without the new flags (`srv cat`, `srv grep`, `errors`, `swarm`, `auth`, `bridge`, `--raw`, `--verbose --level`) are byte-identical to main.

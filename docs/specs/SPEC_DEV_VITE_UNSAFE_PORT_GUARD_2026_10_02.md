# SPEC: A dev window must never be pointed at a port Chromium refuses to load

**Date:** 2026-10-02
**Status:** implemented — all three layers, in the PR that adds this file.
**Author:** AgentX (narko), at the owner's request
**Affects:** `Taskfile.yml` (`dev:serve`, `dev:standalone:serve`), `scripts/vite-port.sh` (new), `scripts/vite-port.test.sh` (new), `.github/workflows/ci-pr.yml`, `.claude/skills/run/SKILL.md`, `crates/cef/src/client/error_catalog.rs`, `crates/cef/src/client/navigation.rs`
**Builds on:** `.claude/skills/run/SKILL.md` (the dev-launch skill whose port step caused this), `docs/retro/RETRO_AGENT_TASK_DEV_LIVENESS_AND_PORT_COLLISION_2026_09_27.md` (the earlier port-collision incident the skill was written from)

## 1. What happened

An agent started `task dev` with `AGENTMUX_VITE_PORT=6000`. Vite came up and answered `200` on `http://localhost:6000/`, the launcher and host started normally, and the window then repainted over and over for more than two minutes and never showed the app. Nothing on screen or in the terminal said why.

The host log for that instance (`<data dir>/logs/agentmux-host-v<ver>.log.<date>`) has the cause, 211 times between the first load at `10:32:47Z` and the kill at `10:35:04Z`:

```
[load-error][ENTRY] url="http://localhost:6000/?ipc_port=…" error="ERR_UNSAFE_PORT" (-312) is_main_frame=true
```

Chromium keeps a list of ports it refuses to load pages from, to stop a web page from talking to services such as SSH or IRC through the browser. Port 6000 is on it, because it is the X11 port. AgentMux's host embeds Chromium, so every page it loaded on 6000 (the main window, the pooled windows, the pooled panes) failed before a single byte was requested.

The first `task dev` of that session ran on 5300 and loaded all 13 of its pages cleanly. The only difference was the port.

### 1.1 Why the window kept repainting

`render_load_error_html` (`navigation.rs`) builds the error page for a failed main-window load. For the main window it adds `setTimeout(__amxRetry, 1200)`, so the page navigates back to the failed URL every 1.2 seconds. That retry exists for a real reason: the host often starts before Vite has finished, and `ERR_CONNECTION_REFUSED` clears itself within a second or two.

`ERR_UNSAFE_PORT` never clears. The retry therefore ran forever, each pass flashing the error page and the app's background, which is the "paint kept resetting" the owner saw. The page's own text was the generic "Failed to load AgentMux frontend" with a hint to run `task dev`, which the user had just done, so it pointed the wrong way.

### 1.2 How 6000 was chosen

The `agentmux:run` skill tells agents to pick a free port themselves and pass it in. Its recipe starts at 5999 and counts up. It has two faults:

- **The probe gave a wrong answer.** It uses bash's `/dev/tcp`, which reported 5999 free while another agent's Vite held it. Git Bash on Windows does not reliably support it. The walk then moved to 6000.
- **It starts next to a blocked port.** Even with a correct probe, any walk upward from 5999 lands on 6000 as soon as 5999 is taken.

The automatic port in `Taskfile.yml` (5173 plus a per-clone offset of 0 to 199, so 5173 to 5372) is not affected: no blocked port lies in that range. Only a manual override can reach a blocked one.

## 2. Requirements

1. A blocked port is refused before anything starts, with a message that says which port, why, and what to do.
2. The skill's port recipe cannot return a blocked port, and its free-port check works on every platform the Taskfile supports.
3. If a blocked port still gets through (a different launch path, a future entry in Chromium's list), the window says so plainly, and does not retry a failure that cannot clear.
4. The list of blocked ports lives in one place, with a test.

## 3. Design

### 3.1 Layer 1 — `scripts/vite-port.sh`

One script owns the list and the checks, so the Taskfile, the skill and the tests cannot drift apart.

| Command | Meaning |
|---|---|
| `vite-port.sh blocked <port>` | Exit 0 if Chromium refuses the port. |
| `vite-port.sh check <port>` | Exit 0 if the value is a whole number in 1024–65535 and not blocked. Otherwise print one error naming the problem and the fix, exit 1. |
| `vite-port.sh listening <port>` | Exit 0 if something is listening on it. Uses `netstat -ano` on Windows and `lsof` elsewhere, the same tools `Taskfile.yml` already uses for its busy check. |
| `vite-port.sh pick [start]` | Print the first port at or above `start` (default 5300) that is allowed and not listening. |

The blocked list is Chromium's restricted-ports table (`net/base/port_util.cc`), limited to the ports from 1024 up, since lower ports are already refused by the 1024 floor. As recorded in this spec at the time of writing: 1719, 1720, 1723, 2049, 3659, 4045, 4190, 5060, 5061, 6000, 6566, 6665 to 6669, 6679, 6697, 10080. Chromium has added entries to this table over the years (4190, 6679 and 6697 are recent), so the list is a snapshot, which is why layer 3 exists.

`Taskfile.yml` calls `check` once, right after the port is settled, in both `dev:serve` and `dev:standalone:serve`:

```
bash scripts/vite-port.sh check "$AGENTMUX_VITE_PORT" || exit 1
```

The check sits in the task's serve step, so it runs after the build steps (about 90 seconds on a warm cache) and before Vite or the launcher start. Moving it ahead of the build would need a second call site in the task's dependency list; left for later since nothing is launched in the meantime. It runs for the automatic port too. That costs nothing and means a future change to the 5173 base or the offset range cannot walk into the list unnoticed. The test also asserts every automatic port from offset 0 to 199 is allowed.

### 3.2 Layer 2 — the skill

`.claude/skills/run/SKILL.md` §2 now says:

- Prefer no override at all. The Taskfile's per-clone port is deterministic, avoids the blocked list, and its own reaping handles an orphan of the same clone.
- If a free port is needed (another agent holds the clone's port), run `bash scripts/vite-port.sh pick` and pass the result. The `/dev/tcp` recipe is removed, with a note on why.

### 3.3 Layer 3 — the window says what is wrong

Two changes in the host's load-error page:

- **Named error.** `error_catalog::describe` gets an entry for `ERR_UNSAFE_PORT`: title and heading "This port is blocked", detail saying the browser engine refuses to load pages from it.
- **No retry of a permanent failure.** A small predicate, `is_permanent_load_error`, is true for `ERR_UNSAFE_PORT`. For the main window the page then omits the 1.2-second auto-retry and shows a specific dev hint in place of "make sure the Vite dev server is running": restart with `AGENTMUX_VITE_PORT` set to another port, for example `AGENTMUX_VITE_PORT=5300 task dev`. The manual Retry button stays. Every other error keeps today's behaviour exactly, including the auto-retry for `ERR_CONNECTION_REFUSED`.

Browser panes already get a manual Retry only and are unchanged apart from the new wording for this one code.

## 4. Decisions made (and why)

| Decision | Why |
|---|---|
| One script for list, check and pick | The Taskfile runs under go-task's embedded shell, which is easy to get wrong in long inline blocks. A script is testable in CI. |
| Check the automatic port too | The auto range is safe today by arithmetic, not by construction. The check makes that stay true. |
| Only `ERR_UNSAFE_PORT` is "permanent" | It is the one failure observed to loop. Other codes can clear (server starting, network back), and the auto-retry exists for exactly those. Widening the set is a later, evidenced change. |
| Same `Retry` button, no dialog | The window is the right place: the user is looking at it. A native dialog would be a new surface for one dev-only case. |
| List kept as a snapshot, not fetched | Fetching Chromium's source at build time adds a network dependency to `task dev`. Layer 3 covers drift. |

## 5. Tests

- `scripts/vite-port.test.sh`, run in CI next to the other script self-tests:
  - every port in the list is reported blocked, and the ports on either side of each are not;
  - `check` rejects empty, non-numeric, below 1024, above 65535, and a blocked port, and accepts 5173, 5300 and 5372;
  - every automatic port (5173 plus offset 0 to 199) is allowed;
  - `pick` skips a blocked port (starting at 5999 never returns 6000) and skips a port that is genuinely listening (a throwaway listener is opened for the test).
- `crates/cef/src/client/navigation.rs`:
  - the error page for `ERR_UNSAFE_PORT` on the main window has no auto-retry and carries the `AGENTMUX_VITE_PORT` hint;
  - the page for `ERR_CONNECTION_REFUSED` on the main window still auto-retries;
  - a browser pane's page never auto-retries, for either code.
- `crates/cef/src/client/error_catalog.rs`: `ERR_UNSAFE_PORT` is described, with a title that is neither blank nor the generic fallback.

## 6. Out of scope

- Reporting a load error from the host to the launcher or the terminal that ran `task dev`. It would be the better place to say this for someone who is not looking at the window, but it needs a new message across the launcher IPC. Left for a separate decision.
- The blank window between the splash and first paint (`docs/reports/REPORT_SPLASH_TO_FIRST_PAINT_BLANK_WINDOW_GAP_2026_09_03.md`). Unrelated: that is a timing gap on a working load; this is a load that cannot succeed.

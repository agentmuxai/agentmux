# RCA: two agents' `task dev` instances were coupled through one Vite (2026-11-06)

**Status:** implemented. Fixes 1 and 2 below are built (task #27095); the window-side check and title are not.
**Found by:** the operator, who saw two windows titled `Agent2-dnd` and asked why the instances were coupled.
**Written by:** Agent2@narko, with Loap-3@narko (the other side's evidence; Loap-3 built the fix).
**Related:** [RETRO_AGENT_TASK_DEV_LIVENESS_AND_PORT_COLLISION_2026_09_27.md](RETRO_AGENT_TASK_DEV_LIVENESS_AND_PORT_COLLISION_2026_09_27.md) §2 and P2 (the same family; its P2 "real port allocator" was never built).

## TL;DR

Two agents on one host each started `task dev` in their own clone. Each window talked to its **own backend** but loaded its frontend from **one agent's Vite dev server**. The window that was not that Vite's owner ran the other agent's frontend code against its own backend.

It happened twice in an hour, with two different ports (5301, then 5300). Three gaps line up, and all three have to be fixed for it to stop:

1. `scripts/vite-port.sh pick` only prints a free port. It reserves nothing, and the port is not bound until much later.
2. `task dev` waits for the shared build slot between the pick and the bind. That wait was about 17 minutes the first time. When the slot frees, queued runs resume together.
3. The readiness check in `dev:serve` is `curl $VITE_URL`. It proves that *something* answers on the port, not that the Vite *this run started* is the one that answers. A Vite that loses the bind (`--strictPort`) dies quietly in the background, and the run carries on against the winner's server.

## What happened (times PT, 2026-11-05 / 11-06)

| Time | Event |
|---|---|
| 23:31 | Loap-3 runs `AGENTMUX_VITE_PORT=$(vite-port.sh pick) task dev`; pick returns 5301 (their 5300 was taken by Korp while they waited, which `task dev` refused correctly). The run then waits on the build slot behind another agent's cargo test. |
| ~23:40 | Agent2 runs the same recipe; 5301 is still unbound, so pick returns 5301 again. Also waits on the build slot. |
| ~23:48 | The slot frees. Both runs finish their builds and reach `dev:serve` together. Both pass the "port busy" check (nothing is bound yet), and both start `vite --strictPort` on 5301. |
| 23:48 | Agent2's Vite binds first ("Vite ready"). Loap-3's Vite prints `Error: Port 5301 is already in use` and exits, but the `curl` readiness loop passes against Agent2's Vite and prints "Vite ready at http://localhost:5301". Both launchers start (cef pids 47032 and 44996, started one second apart), both with `--url=http://localhost:5301`. |
| 23:48 to 01:44 | Loap-3's window (renderer lab, branch `loap-3-renderer-lab-run`) runs Agent2's frontend (branch `agent2/dnd-refine`: main plus the two tear-off PRs) against Loap-3's own srv. Agent2's window is fine. |
| ~01:3x | The operator sees two `Agent2-dnd` windows. Agent2 stops its own instance, Loap-3's window is left dead (its Vite server is gone), and Agent2 relaunches. |
| 01:44 | Loap-3 relaunches on 5300 (their 5301 instance already stopped). |
| ~01:50 | **Second occurrence.** Agent2's relaunch also picks 5300 (free again) and both builds again finish together: their `dist/cef-dev-<stamp>` folders have the **same stamp, 1793958529**. Agent2's Vite wins 5300, Loap-3's cef (pid 43988) loads it. Reproduced on demand, with both sides still running. |

## Evidence

- `.dev-home/5301/dev/loap-3-renderer-lab-run/.../data/authkey.dev` in Loap-3's clone, written 23:48, host pid 44996. That pid's command line is `...loap-3-0822g\agentmux\dist\cef-dev-1793951304\runtime\agentmux-cef.exe --url=http://localhost:5301`.
- The only listener on 5301 was node pid 38800, command line `...agent2-0630f\agentmux-devmode\node_modules\vite\bin\vite.js ... --port 5301 --strictPort`.
- Loap-3's task output, last 500 bytes: `(client) Re-optimizing dependencies because lockfile has changed`, then `error when starting dev server: Error: Port 5301 is already in use`, then `Vite ready at http://localhost:5301.` (their own Vite failed, the "ready" line still printed).
- Second occurrence: cef 20852 (Agent2) and cef 43988 (Loap-3), both `dist\cef-dev-1793958529`, both `--url=http://localhost:5300`; listener on 5300 is Agent2's node pid 55540.

## Root cause

`Taskfile.yml` `dev:serve` (the Windows branch, around the `vite ... --strictPort &` line):

- The port is chosen by the caller (`vite-port.sh pick`) before the run starts. The Taskfile's own check (`netstat ... LISTENING`) runs again late, but only to ask "is it bound *now*". Two runs that reach it in the same second both see "free".
- `npx vite ... --strictPort &` is started in the background. `--strictPort` makes the loser fail instead of drifting to another port (its comment says this closes the TOCTOU window), but the loser's failure is never observed: `$!` is not usable under go-task's shell (the Taskfile says so), and the next step is `curl -s "$VITE_URL"`, which succeeds against the winner.
- The cef host is given `--url=$VITE_URL`, so it loads whichever Vite answers.

The comment on `--strictPort` assumes the readiness loop would otherwise attach to a foreign Vite and that `--strictPort` prevents that. It does not: without a liveness check on its own Vite, `--strictPort` only makes the loser's Vite exit; the loser's window still attaches to the foreign one.

## Impact

- Loap-3's renderer-lab window ran Agent2's frontend from 23:48 to 01:44. **Anything observed in that window is not an observation of Loap-3's branch.** The lab had not yet reached its measurement run, so no figures were taken from it; it is re-run on the fixed task dev.
- Agent2's window was unaffected, except that Loap-3's window's HMR connection and requests were also served by its Vite.
- Not a data problem: each instance keeps its own `.dev-home/<port>` in its own clone, and the backends were never shared. Only the frontend source was.

## The fix (built)

1. **Claims at pick, and again at start.** `scripts/vite-port.sh` has `claim <port>` and `release <port>`, and `pick` now claims what it prints, in the same step. A claim is a directory made with `mkdir` (atomic) under `~/.agentmux/dev-port-claims` (`AGENTMUX_DEV_PORT_CLAIMS`), naming the clone and agent that took it. A second picker skips the port; a second `claim` on it exits 1 and names the holder. The same clone renews its own claim, so `pick` then `task dev` agree. A claim nobody renewed for two hours (`AGENTMUX_DEV_PORT_CLAIM_TTL`, longer than the build slot's one-hour wait) is taken over, under a lock so two takers cannot each delete the other's. `task dev` and `task dev:standalone` claim the port again right after the check, so the hash-derived and hand-set ports are covered, and release it when the app exits.
2. **Check your own Vite, not the port.** `task dev` gives each run a token (`AGENTMUX_DEV_INSTANCE`). `vite.config.ts` loads `scripts/dev-instance-plugin.mjs`, which makes the dev server answer `/__agentmux_dev_instance` with that token, its owner and its commit. `scripts/dev-vite-wait.sh` replaces the `curl` loop and passes only on our token. Any answer without it is another run's server (our own Vite cannot answer with the wrong token, and before it binds nothing answers), so it stops at once and prints what answered. A Vite from before this change answers 404, which is also a refusal.
3. **The start log names the owner.** `task dev` prints `[dev] instance: owner <agent>, commit <sha>, port <n>, clone <path>`.

Tested in `scripts/vite-port.test.sh` (run by CI): 12 simultaneous picks get 12 different ports, a held port is refused with the holder named, a lapsed claim is taken over, and the wait passes a server with its token, refuses one with another token at once, and fails on a silent port. With the claim removed from `pick`, or the token check removed from the wait, those tests fail. Replayed live with two real Vites on one port: the old `curl` check got HTTP 200 from the winner; the new wait refused in 2 s and printed the winner's owner and commit.

**Not built, for a follow-up:**

- **The window checks its own server.** The cef host would carry the token in its URL or ask for it before loading, so a window that was handed the wrong URL refuses too. This needs a host (Rust) change; the task-level check above already stops the run before the window starts.
- **The window title names the owner and commit** (today only the start log does).
- **`srv` and CDP ports.** Not checked here. The data home is per `.dev-home/<vite port>`, so two runs on different Vite ports have different homes; whether their srv and debugging ports can collide is open.
- **A clone under `subst`.** The claim names the clone by `pwd -P`, which does not expand a `subst` drive, so the same clone reached by two paths looks like two clones. It fails loudly, naming itself.

## What each of us did

- Loap-3 and Agent2 each stopped only their own processes, launcher first, by confirmed image path. Neither touched the other's processes.
- The second occurrence was ended the same way: Loap-3 stopped its instance (launcher first, by confirmed path) and relaunched.

Agent: Agent2@narko, finished by Loap-3@narko

# Spec: Windows Headless as a real service: starts at boot, survives sign-out, installed by one documented command

**Status:** approved, revised 2026-11-20 with the security review's answers (the three questions answered yes, with the conditions now written into B.1, B.3, C, D.2 and F). Written against agentmux `origin/main` 351954ee87 (2026-11-20).
**Requirement (restated):** a person can install AgentMux Headless on Windows, have it start as a Windows service with nobody signed in (and keep running after they sign out), and open a browser that is connected to it.
**Sources:** the code (`crates/srv/src/agentmux_cli/service.rs`, `setup.rs`, `crates/srv/src/agentmuxw.rs`, `crates/srv/src/server/local_session.rs`, `crates/srv/src/headless.rs`), `docs/specs/SPEC_HEADLESS_RENAME_AND_PER_PLATFORM_2026_11_16.md` (sections 5.1 and 6), and a test of the published 0.60.36 on a Windows test host. **Nothing in this spec was run on Windows by its author.** Statements about how Windows behaves that the code and Posa's test do not show are marked *(to prove)*: step 1 of the build (section G) proves each on a real machine before anything depends on it.

## 0. Decisions (numbered, with a recommendation)

| # | Decision | Recommendation |
|---|---|---|
| D1 | How Windows starts Headless with nobody signed in | **A real Windows service**, run by `agentmux` itself (`agentmux service install / uninstall / start / stop / status`). Not a boot-time Scheduled Task, not a third-party wrapper (B.2). |
| D2 | Which account runs it | **A dedicated virtual service account** (`NT SERVICE\AgentMuxHeadless`) with its **own data folder under `%ProgramData%`**, not the person's profile. Running as the person's own account (`--run-as-me`) is **not in the first build**: it needs a password stored by Windows and exposes the person's whole fleet to a browser sign-in. It stays a later decision (B.3). On a **domain-joined** machine the service refuses unless `--allow-domain-joined` is given (C.6). |
| D3 | The default view of the machine's agents | **Empty.** A service starts with no agents, no accounts and no tasks of anyone's; the person adds what they want through the browser. It never reads the person's `%USERPROFILE%\.agentmux`. (If `--run-as-me` is ever added, `setup`, `doctor` and the first page must say in words that a signed-in browser sees the whole fleet: D.1.) |
| D4 | Browser sign-ins across a restart | **Keep them.** Today every restart signs every browser out (memory only). A service restarts at boot and at every update, so persist the session digests, owner-only, with the same 12 hour idle and 7 day maximum; `agentmux logout` (which already ends every session), a changed key and uninstall clear them (D.2). Needs the security review's sign-off: it reverses the "memory only" rule of `local_session.rs`. |
| D5 | The program folder | A service runs from a folder **only administrators can write**: `%ProgramFiles%\AgentMux\Headless`. This replaces, for the service, rule 5 of the 2026-11-16 spec (program inside the person's profile). The logon task stays as it is for people who do not want a service (B.4). |
| D6 | The elevation | One UAC prompt, a person's gesture, at `agentmux service install` (and at uninstall and update of a service). An agent cannot click it; a test host's owner does (C). **The elevated step never runs code from a user-writable path** (B.1): the files are copied to Program Files first, verified there against the signed manifest, and the elevated relaunch is of that copy. |

Rules 1 to 4 and 6 to 7 of the 2026-11-16 spec's section 6 stay as they are (loopback only, exclusive port, server proof for `open`, no streamed browser on Windows, manifest-checked downloads). Rules 2 and 5 change for the service, as D5 and B.3 say; the security review decides that (it settled section 6).

## A. What exists today, per OS

| OS | What starts it | Starts before sign-in? | Survives sign-out? | The docs say |
|---|---|---|---|---|
| Linux | a systemd **user** unit (`agentmux.service`, `WantedBy=default.target`), written by `agentmux setup --service` | only with `loginctl enable-linger $USER` (`setup` prints the command; it does not run it) | with linger | "Start at login" |
| macOS | a launchd **agent** (`ai.agentmux.server`, `RunAtLoad`, `KeepAlive`) in `~/Library/LaunchAgents` | no: it loads at the person's login | yes, while they stay logged in; no after a restart until login | "Start at login" |
| Windows | a per-user **Scheduled Task** `AgentMux Headless`, `LogonTrigger`, `InteractiveToken`, least privilege, runs `agentmuxw.exe --channel headless` (a no-console program that starts `agentmux.exe run` and waits for it); registered by `agentmux setup --service` with `schtasks /Create /XML` | **no**: it starts when the person signs in | the task is "run only when the user is logged on" (`InteractiveToken`), so a sign-out ends it *(to prove on a Windows test host)* | "It is not a Windows service, which would start before you sign in, as another account" |

What the test of 0.60.36 on a test host showed without running `setup`: the install from the release is clean (signature and manifest checked, no admin); `agentmux start` runs it with no console on `127.0.0.1:7380`; `agentmux doctor` says `service: none`; both ports listen on loopback only and refuse the machine's LAN address; not signed in, `/ws` answers 401 and the loopback web port 7381 answers 403. So the logon task itself has **no proof on a real machine yet** (no output, no sign-out or reboot result): step 1 of the build produces it, on a test host with its owner's agreement.

## B. The Windows options

### B.1 What a Windows service needs that the logon task does not

- **An entry point that speaks to the Service Control Manager (SCM).** SCM starts the program and waits for it to report "running" (and "stopped" on a stop request); a console program that does not do so is killed after about 30 seconds. `agentmuxw.exe` does not speak it. The wrapper is a new small mode of the binary we already ship (`agentmux service run`, a `windows-service` style main) that starts `agentmux run`'s server in its own process, reports status, and turns SCM's stop into the server's existing graceful shutdown. No new third-party program is shipped. *(The repo has no service crate today; adding one is a dependency decision for the build's PR, reviewed like any other.)*
- **An account, and a place for its files** (B.3).
- **A program folder only administrators can write** (D5): a service runs its program as another account, so a program a normal user can replace is a way to run code as that account. This is the reverse of the logon task's rule (a task runs as the person, so the program must be in *their* profile and nobody else's).
- **An elevated install.** Creating a service, and creating a boot-triggered task, both need an administrator's token *(to prove for the task)*.
- **The elevated step must not run anything a normal user can swap.** The `agentmux.exe` the person types is in `%LOCALAPPDATA%`, which any process running as that person can replace before the UAC click. So `service install` does, in order, as the *unelevated* command: (1) check the package it runs from against the signed manifest (the check `install.ps1` already makes); (2) ask for elevation by relaunching **not itself** but a minimal elevated step that does the rest; and that step (3) copies the package files to `%ProgramFiles%\AgentMux\Headless\<version>` (a folder only administrators can write, checked up the whole tree, B.3), (4) **hashes the bytes in the destination**, not the source, against the signed manifest, and refuses on any mismatch, (5) moves the `current` junction, and (6) from then on runs and registers only the Program Files copy. The relaunch itself is of the destination copy once step 4 has passed; an elevated process never executes the user-writable binary. (How to make the first elevated launch safe, since the relaunch needs *some* program to start elevated, is part of PR 1's proof: the candidates are a manifest-pinned copy made by the unelevated command into a temporary admin-only folder, or Windows' own `msiexec`/`Start-Process -Verb RunAs` of the already-copied destination. The build does not choose until PR 1 shows which works with a UAC prompt that names the right program.)

### B.2 The three ways

| | Real service (D1) | Scheduled Task with a boot trigger | A wrapper (NSSM and the like) |
|---|---|---|---|
| Starts before sign-in, survives sign-out | yes | yes, as a task "run whether the user is logged on or not" *(to prove)* | yes |
| Standard tooling (`services.msc`, `sc`, Event Log, recovery actions) | yes | no: Task Scheduler, no service-style recovery beyond `RestartOnFailure` | partly |
| Elevation | once at install | once at install *(to prove)* | once |
| New code | the SCM entry (B.1) and `service` commands | small: reuse `install_task`/`agentmuxw.exe` with a boot trigger | none, but we would ship and update a program we do not own and cannot sign |
| Account | virtual service account, LocalService, or a named account with a password | a named account; "without a stored password" (S4U) cannot reach the network or the Credential Manager *(to prove)* | as the service |
| Verdict | **recommended** | the fallback if the SCM entry proves costly: cheaper, less standard | rejected: a supply-chain and update cost for no gain |

### B.3 The account, the profile and what an agent can do under it

A service has **no signed-in session**: no desktop, no per-user HKCU hive loaded for a virtual account or LocalService (its own profile only), **no access to the person's Credential Manager or DPAPI secrets**, its own PATH (the machine's, not the person's shell's), and `%USERPROFILE%` is the account's (for LocalService a folder under `C:\Windows\ServiceProfiles`; for a virtual account its own, *(to prove which)*). Consequences, each with the default it gets:

| Thing | Under the service account | Default |
|---|---|---|
| Data root | `dirs::home_dir()` is the account's profile | `service install` sets `AGENTMUX_HOME_OVERRIDE=%ProgramData%\AgentMux\Headless` (already supported by srv), created with an access list naming **only the service account and Administrators** (not the installing person: they do not need to read API keys or sessions); no dependence on the account's profile. `service install` refuses when any folder up the tree to the root is writable by a non-administrator other than the service account |
| Secrets (API keys, accounts) | no Credential Manager | the existing owner-only **file** store (`--secret-store file`, already the headless default); the files are under the data root |
| Key file (`srv-auth-key`) | must be readable by the service **and** by the person who will run `agentmux open` | the one file whose access list also names the installing person's SID (recorded at install): that is what `open` needs; plus the service account and Administrators. `doctor` checks it as the owner-only check is today |
| Agent CLIs (Claude Code, git, npm, `gh`) | not the person's: their logins live in their profile and Credential Manager | none. The person signs each agent in through the browser, inside the service's own home. `service install` prints that sentence. Machine-wide tools (Git for Windows, Node in `%ProgramFiles%`) are on the machine PATH and found |
| Network | outbound works for a virtual account (as the computer) | unchanged |
| What an agent in it can do | run commands and read/write files **as the service account**, nothing of the person's | this is the isolation D3 buys; it is also why a service account is **not** LocalSystem and never an administrator (no SYSTEM, no admin rights for the account) |
| Running as the person (`--run-as-me`) | their profile and tools, but Windows needs their **password** to start a service as them (stored by SCM in the LSA), and a signed-in browser then sees their whole fleet (D.1) | **not in the first build.** A later decision, with its own spec section |

### B.4 What stays

The per-user logon task stays, unchanged, for a person who wants Headless to run only while they are signed in and to see their own profile. `setup` offers both (`--service` = the logon task as today; `agentmux service install` = the real service); `doctor` says which one is installed and flags both at once.

## C. Safe by default

1. **Loopback only.** The service listens on `127.0.0.1:7380` (the next port if taken), exclusive bind, like the foreground server. It never listens on the network. Reaching it from another machine is a reverse proxy or an SSH tunnel (`--public-origin`, existing), a decision the person makes per run, not a service default.
2. **Sign-in required for everything but `/health` and the static page**, as today (the test of 0.60.36: `/ws` 401, 7381 403 when not signed in). Nothing about the service weakens it.
3. **The elevation is the person's gesture.** `agentmux service install` relaunches itself elevated (UAC, "Run as administrator"); an unattended or agent-driven install is refused, the same line a human reads: "this needs an administrator: run it from a PowerShell opened as Administrator". Nothing installs a service silently, and `install.ps1 -Headless` (no admin) does not install one.
4. **Files:** program folder admin-write only; data root and key file as in B.3; `doctor` checks all three and `service install` refuses when a folder is writable by an account that should not (the refusal text of `owner_only::refusal_because`, reused).
5. **Recovery actions** (restart after a failure) are set to restart twice and then stop, so a crash loop is visible in the Event Log and not a hot loop.
6. **Domain-joined machines.** A virtual service account authenticates on the network as the computer account, and an agent in it runs commands. `service install` detects a domain-joined machine, warns in words, and refuses unless `--allow-domain-joined` is given; the docs say why.
7. **Nothing secret in a log.** The Event Log events and `server.log` never contain a one-time code, check phrase, cookie, token or key (a test greps the logs after a full sign-in, a restart and a failed sign-in). The environment the service gives its agents is built from the service account's own and the machine's, never inherited from the installer's shell (a test starts the installer with a marker variable and asserts an agent does not see it).

## D. How a browser connects and signs in

### D.1 The sign-in flow with a service

Today `agentmux open` (a terminal, run as the user who started the server) mints a one-time code with the key file, the browser enters it, both show a check phrase, the person answers `y`. This does not need the server's own account: it needs the key file and a terminal. With the service:

- The person who ran `service install` runs `agentmux open` in a normal (non-elevated) terminal. The key file's access list (B.3) lets their account read it, so the flow is unchanged, including the server proving it holds the key and is bound to the address before anything is sent (rule 4 of the 2026-11-16 spec).
- On a machine where **no one is signed in** (the point of a service), the person signs in over RDP or SSH, runs `agentmux open --print`, and reaches the page with the existing tunnel (`ssh -L 7380:127.0.0.1:7380`). Said in the docs as one short section.
- A person with no key-file rights (another account on the machine) cannot sign a browser in; that is the property wanted.
- **What a signed-in browser sees:** with D3's default, an empty AgentMux (no agents, accounts or tasks). With `--run-as-me`, **the person's whole fleet** (a test of 0.60.36 run as the same user as the desktop showed the desktop's agent definitions with their last prompts, its Swarm, its Kanban tasks, and log lines about other agents' memory files in the shared folder). Reading a fleet is the point of sharing it, but it means anyone who signs a browser in sees it all, so `--run-as-me` says so at the prompt, `doctor` shows `data: shared with <user>'s desktop`, and the page's status bar names the account the server runs as.

### D.2 Browser sign-in across a restart (D4)

Today a restart ends every code, pending session and session (`local_session.rs`: memory only), which the test saw (`browsers: none signed in`, "This browser's sign-in has ended"); the docs page promises 12 h idle / 1 week and does not say a restart ends it. A service restarts at boot and at every update; each would sign every person out.

Proposal: sessions are stored as the **digests they already are** (SHA-256 of cookie and per-tab token), with their idle and absolute expiry, in `<data root>\sessions.bin`, owner-only like the key file, written when a session is confirmed, used or ended, loaded at start. The per-tab token lives in the browser's `sessionStorage`, so a closed tab still needs a new sign-in; the cookie alone proves little (existing design). Cleared by `agentmux logout` (it ends every session today), by a new key file, by `service uninstall --purge`, and at expiry. Pending sessions and one-time codes stay memory-only (60 seconds). Until the security review signs off, the docs line is corrected instead: "a restart signs every browser out".

**Conditions (security review, 2026-11-20), all required and each tested:** tokens and cookies are at least 128 bits of randomness; only SHA-256 digests are stored, compared in constant time; `sessions.bin` lives under the data root and is written by temp file and rename; at load the file's access list is checked, and a file that is too open, truncated or unparsable is **ignored and every session cleared, never failing open**; the absolute expiry is stored as wall-clock time and enforced on both the idle (12 h) and the absolute (7 day) limits, **also when the clock jumps back**; cleared by `logout`, a key change, `uninstall --purge` and expiry; pending sessions and one-time codes stay memory-only. Tests: tamper, too-open access list, truncated file, expired, key change, clock set back.

## E. Uninstall, update in place, logs

- **Uninstall:** `agentmux service uninstall` (elevated) stops and deletes the service, keeps the data root and the program folder's version folders; `--purge` also deletes the data root and `sessions.bin` after a confirmation panel in the terminal. `agentmux uninstall` (existing) refuses while a service uses the install, as it refuses for the logon task (the message it already has, with the service named).
- **Update in place:** the service's program path is `%ProgramFiles%\AgentMux\Headless\current\bin\agentmux.exe` (a junction the installer moves, as today under `%LOCALAPPDATA%`). A running program's files are locked, so an update is: stop service, replace the version folder, move `current`, start service; `install.ps1 -Headless` does it when it sees a service and has an administrator token, otherwise it installs the version, does not touch the service, and prints the exact elevated command (`agentmux service update`). The session digests (D.2) mean the person stays signed in across it.
- **Logs:** `server.log` under the data root, as `agentmux logs` reads today; a service has no stdout, so the SCM wrapper also writes start, stop and failure events to the Windows Event Log (source "AgentMux Headless"). `doctor` reports the service's state, its account, the program path and the last start.

## F. Docs, and how it is proved

- **Docs** (agentmux-docs repo, `headless-server.md`, "Start at login"): also say that a Headless started by hand or by the logon task **shares the person's `shared\` folder with the desktop** (the whole fleet, B.3/D.1), and that `AGENTMUX_HOME_OVERRIDE=<folder>` moves the whole root, `shared\` included (proved on a test host), which is how a person gets an empty, separate Headless without a service; the service uses exactly that (B.3, data root). Then: replace "It is not a Windows service" with the two choices (logon task, service), the elevated command, the account and what it can see (D3), the sign-in on a machine with nobody signed in (D.1), update and uninstall. `agentmux setup`'s end-of-run text and `doctor` carry the same sentences.
- **Tests:** unit tests for the service definition (name, account, program path, arguments, access lists rendered by a pure function, as `render_task_xml` is tested), for the refusals (non-admin, program folder writable by non-admins, data root access list), and for the session file (round trip, expiry, tamper, `logout`). A Windows-runner test installs the service on a hosted runner (which is an administrator), starts it, asks `/health` on loopback, checks the exclusive bind, stops and uninstalls. Also: the elevated-copy check (B.1: a package whose destination bytes differ from the manifest is refused, and a source swapped after the check is caught because the destination is hashed), the domain-joined refusal (C.6), and the log and environment tests (C.7).
- **Real proof (a Windows test host):** needs an administrator's UAC click, so the host's owner is told first: (1) install from the release, (2) `agentmux service install`, (3) restart the machine with no one signed in, (4) from a fresh sign-in `agentmux status` shows the service running since boot, (5) `agentmux open`, drive a Browser pane to the page, (6) sign out and sign in again, the service still runs, the browser is still signed in (D4), (7) `agentmux service uninstall`. A host whose account must not run `setup` cannot be used: the proof runs on a host that allows it or a spare account or VM; the build does not start the proof until a host is agreed.

## G. Effort and order

| PR | What | Size |
|---|---|---|
| 1 | **Proof of the facts marked *(to prove)***, no product code: on a test host, register by hand a boot task and a service with `sc` (virtual account, `%ProgramData%` home), start the server under both, record the profile folder, PATH, secrets, outbound network and sign-out result. Written into this spec | small, a day |
| 2 | The logon task proof (`setup --service` run once, sign out, reboot) and the docs correction for D4's "restart signs you out" | small |
| 3 | `agentmux service run` (SCM entry, graceful stop) and `service install / uninstall / start / stop / status` with the program-folder, data-root and key-file access-list checks, elevation relaunch, Event Log | the main PR, large |
| 4 | Session persistence (D.2) | medium; after the security review's approval |
| 5 | `install.ps1 -Headless` sees a service, the update path; `doctor`; `agentmux uninstall` refusal text | medium |
| 6 | Docs (agentmux-docs), the Windows-runner test, the real proof (section F) | small plus the proof |

Order: 1, 2, then 3 and 4 in parallel, then 5, 6. The logon task is the first step (already shipped); the real service is the next. A boot-triggered task (B.2) takes over from PR 3 only if PR 1 shows the SCM entry is not worth its cost. Each PR carries a changeset and is merged on muxreview approval; the runs on test hosts need no further approval.

## Security review answers (2026-11-20)

1. Rules 2 and 5 for the service: yes, on the conditions written into B.1 (the elevated step runs only a verified Program Files copy), B.3 (data root and key file access lists) and C.4.
2. Session digests on disk: yes, on the conditions in D.2.
3. A virtual service account: yes, never LocalSystem and never an administrator; domain-joined machines refuse by default (C.6); `--run-as-me` is out of the first build (B.3).

PR 1 (the proof on a test host, no product code) may start now with the host owner's agreement.

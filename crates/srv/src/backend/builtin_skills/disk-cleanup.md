# Disk cleanup for AgentMux

Use this when the user asks to free disk space, or the disk is nearly full and AgentMux's own files may be part of it. Work in three steps: measure, propose, and only then delete, and only what the user approves.

## Rules that never bend

1. **Ask before deleting anything.** Showing a list is not an OK. Get the user's explicit yes for each item (or each group they name), and delete exactly what they approved.
2. **Never touch** any of these:
   - the live channel's data (see "Find out what is live"), and in any channel folder its `agents/`, `config/` and `identity-store.db`;
   - `db/` (the live databases and their `-wal` and `-shm` files);
   - the `shared/` folder in the AgentMux home: the stores, identities, vault, memory, the conversation (transcript) store, audit logs and backups of history;
   - any agent's memory files;
   - agent workspaces (`agents/<name>/`): they hold the user's work, clones and uncommitted changes;
   - `widgets/`, and the user's settings and config (`settings.json`, `config/`);
   - anything a running process is using.
3. **Only remove a folder when nothing is running from it.** If you cannot tell, do not remove it.
4. **If a folder is not in the table below, or you do not know what it is, do not propose deleting it.** Say what you found and leave it.
5. Print the full absolute path before removing anything. Never build a path from a variable that might be empty, and never use a wildcard on a parent folder.

## Find out what is live

- Find the AgentMux home and **confirm it with the user before measuring or deleting anything**. An isolated copy (a test or development instance) has its own home, and an agent's environment may not carry `AGENTMUX_HOME_OVERRIDE`, so an unset variable does not prove the default `.agentmux` in the user's home folder is the right one. Work it out from the running copy: `AGENTMUX_AGENT_WORKDIR` (your workspace) sits under `<home>/agents/` or `<home>/channels/<channel>/agents/`, the command line of the running `agentmux` processes shows the folders they use (see below), and `AGENTMUX_HOME_OVERRIDE` is the answer when it is set. Say which folder you mean and ask if it is the one to clean.
- The running version and channel are in `AGENTMUX_VERSION` and `AGENTMUX_INSTANCE_CHANNEL`. The live data is in `<home>/channels/<channel>/versions/<version>/`, and the channel folder `<home>/channels/<channel>/` also holds what outlives versions: `agents/` (agent definitions), `config/` (settings) and `identity-store.db`. The installed app's channel is `stable`; channels named `local-...` come from locally built or development copies. Your own session may be running in one of them: that channel is live.
- Other AgentMux copies may be running at the same time (other windows, test builds, a portable copy kept anywhere on the disk), so a program's own path tells you nothing. Check the **command line** of every process for the folder you want to remove (the browser engine's helper processes carry `--user-data-dir=<channel>/versions/<version>/cef-cache`). On Windows: `Get-CimInstance Win32_Process | Where-Object CommandLine -like '*<folder>*' | Select-Object ProcessId, Name`; elsewhere `ps -eo pid,args | grep -F '<folder>'`. Each channel's `agentmux-last-version.json` names the version it last ran.
- Folder layouts change between versions. List a folder before you act on it; do not assume.

## Measure first (read only)

- Check the free space on the drive first and say it.
- Measure one level at a time. A large tree can take minutes to scan, so run the scan in the background and report as results arrive.
  - macOS, Linux, Git Bash: `du -sk <folder>/* | sort -n`
  - PowerShell: `Get-ChildItem <folder> | ForEach-Object { [pscustomobject]@{ Name = $_.Name; MB = [math]::Round((Get-ChildItem $_.FullName -Recurse -Force -ErrorAction SilentlyContinue | Measure-Object Length -Sum).Sum / 1MB) } } | Sort-Object MB`
- Show the user a table: folder, size, last modified, what it is, and your verdict (safe to remove after an OK / leave).

## What AgentMux leaves behind

In the AgentMux home (paths are relative to it):

| Where | What it is | Verdict |
|---|---|---|
| `channels/<channel>/versions/<older version>/` | One older version of a channel: `data/`, `logs/`, `cef-cache/` and `runtime/` (the program copy that starts that version) | Can go, as a whole, once no process runs from it (command-line check above). Keep the live version, and the newest older one if the user may roll back. `runtime/` goes only with the whole folder, never alone. If unsure about `data/`, remove only the `cef-cache/` and `logs/` of **older** versions |
| `channels/<channel>/versions/<live version>/cef-cache/` | The browser engine's profile: sign-ins and cookies of browser panes | **Leave.** Removing it from the live version signs the user out of sites |
| `channels/local-.../versions/<version>/` | Version folders made by locally built or development copies | Can go when no process runs that copy. Never `stable`. Leave the channel's `agents/`, `config/` and `identity-store.db`; if the user wants a whole local channel gone, name those three, say what they hold, and get a separate OK |
| `channels/<channel>/updates/` | Downloaded installers | Can go once that version is installed |
| `channels/<channel>/` other small files, `channels/settings.json` | Settings, identity links, version markers | Leave. `settings.json` is a file |
| `versions/<version>/`, `instances/<version>/` | Older program or instance folders of earlier layouts. A legacy `versions/<v>/` holds `agents/`, `config/`, `data/`, `logs/`, `cef-cache/`; `instances/<v>/` holds the installed agent CLIs | Look inside before proposing; keep the live version and the one before it. Do not remove a legacy folder that still has `agents/` the user may need without asking about that folder specifically |
| `<version data>/snapshots/*.bak/` (current) and `snapshots/*.bak/` in the home (older builds) | Backups taken before an upgrade or by a development build. Each entry is a folder named with the kind, branch, version and time | Old ones can go (remove the folder and what is in it). Keep the newest one or two of real upgrades: they are the way back. Backups of development builds for versions long gone can all go |
| `dev/<name>/` | Homes of development test instances: their own test databases, not the user's history | Can go when that instance is not running |
| `tmp/`, `tool-build-cache/` | Temporary files and tool build caches | Safe when nothing runs from them; they are re-created |
| `cache/` | Agent scratch files (notes, claims) | Leave unless the user says it can go |
| `cli/`, `shared/cli/` | Installed agent command-line tools | Leave; at most remove old versions inside, with an OK |
| `shared/backups/pre-migration-*/` | Backups taken before a data migration | Keep the last two; older ones can go |
| `shared/backups/history-*` | Copies of damaged history | Ask; they may be the only copy |
| `logs/` (in the home and in version folders) | Old log files | Files older than a few weeks can go |
| `archives/` | Compressed older conversation records | Treat as history: leave them unless the user asks |

Outside the home, agent workspaces often hold build folders (`target/`, `node_modules/`, `dist/`). Report them with their sizes. Removing one is safe but costs a rebuild; it is the user's call, and never touch the rest of the workspace.

## Propose, then act

1. Give the user the table, the total you could free, and a short recommendation. Ask which items to remove.
2. After the answer, remove the approved items one at a time, saying which one and how much it freed. Say whether you are using the recycle bin or deleting permanently.
3. At the end, report the free space again. If something fails or a folder turns out to be in use, stop and tell the user instead of forcing it.
4. If the biggest item is something you must not touch (the live data, the conversation store), say so plainly and suggest what the user can do in the app instead of deleting it yourself.

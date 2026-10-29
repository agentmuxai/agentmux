# Disk cleanup for AgentMux

Use this when the user asks to free disk space, or the disk is nearly full and AgentMux's own files may be part of it. Work in three steps: measure, propose, and only then delete, and only what the user approves.

## Rules that never bend

1. **Ask before deleting anything.** Showing a list is not an OK. Get the user's explicit yes for each item (or each group they name), and delete exactly what they approved.
2. **Never touch** any of these:
   - the live channel's data (see "Find out what is live");
   - the `shared/` folder in the AgentMux home: the stores, identities, memory, the conversation (transcript) store, audit logs and backups;
   - any agent's memory files;
   - agent workspaces (`agents/<name>/`): they hold the user's work, clones and uncommitted changes;
   - the user's settings and config (`settings.json`, `config/`);
   - anything a running process is using.
3. **Only remove a folder when nothing is running from it.** If you cannot tell, do not remove it.
4. **If a folder is not in the table below, or you do not know what it is, do not propose deleting it.** Say what you found and leave it.
5. Print the full absolute path before removing anything. Never build a path from a variable that might be empty, and never use a wildcard on a parent folder.

## Find out what is live

- The AgentMux home is the folder in `AGENTMUX_HOME_OVERRIDE` if that is set, otherwise `.agentmux` in the user's home folder.
- The running version and channel are in `AGENTMUX_VERSION` and `AGENTMUX_INSTANCE_CHANNEL`. The live data is in `<home>/channels/<channel>/versions/<version>/` (data, logs, runtime and cache folders). The installed app's channel is `stable`; channels named `local-...` come from locally built or development copies.
- Other AgentMux copies may be running at the same time (other windows, test builds). Before removing a version folder, a channel, a dev home or an instance folder, list the running processes whose name starts with `agentmux` and check which folders they were started from.
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
| `channels/<channel>/versions/<older version>/` | Data, logs and caches of an older version of a channel | Can go once no process uses it. Keep the live version, and keep the newest older one if the user may roll back. If unsure what is in a `data/` folder, remove only its `cef-cache/` and `logs/` |
| `channels/local-.../` | Folders made by locally built or development copies | Can go when no process runs that copy. Never `stable` |
| `versions/<version>/`, `instances/<version>/` | Older program or instance folders | Check what is inside before proposing; keep the live version and the one before it |
| `snapshots/*.bak` | Backups taken before an upgrade | Old ones can go. Keep the most recent one or two: they are the way back |
| `dev/<name>/` | Homes of development test instances | Can go when that instance is not running |
| `cef-cache/` inside a version folder, `cache/`, `tmp/`, `tool-build-cache/` | Caches that are rebuilt on demand | Safe when nothing runs from them; AgentMux re-creates them |
| `logs/` (in the home and in version folders) | Old log files | Files older than a few weeks can go |
| `archives/` | Compressed older conversation records | Treat as history: leave them unless the user asks |

Outside the home, agent workspaces often hold build folders (`target/`, `node_modules/`, `dist/`). Report them with their sizes. Removing one is safe but costs a rebuild; it is the user's call, and never touch the rest of the workspace.

## Propose, then act

1. Give the user the table, the total you could free, and a short recommendation. Ask which items to remove.
2. After the answer, remove the approved items one at a time, saying which one and how much it freed. Say whether you are using the recycle bin or deleting permanently.
3. At the end, report the free space again. If something fails or a folder turns out to be in use, stop and tell the user instead of forcing it.
4. If the biggest item is something you must not touch (the live data, the conversation store), say so plainly and suggest what the user can do in the app instead of deleting it yourself.

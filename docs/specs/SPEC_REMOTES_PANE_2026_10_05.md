# SPEC: The Remotes pane: one place for every remote machine

**Date:** 2026-10-05
**Status:** proposed
**Author:** Korp (narko), at the owner's request:
- "should we have a pane called something like 'Remotes' .. those are the connections that are set, and then inside a terminal or file browser, there would be an interface to select the remote";
- "lets use Remotes .. write the spec to file".

**Affects:**
- `frontend/app/view/remotes/` (new);
- the pane registry (`block/block-registry.ts`, `block/pane-tab-registry.ts`);
- the connection picker (`modals/conntypeahead.tsx`);
- the block frame;
- Hangar (`view/files/`);
- `crates/srv/src/backend/remote/`;
- `crates/srv/src/server/app_api/connections.rs`.

**Builds on:** `SPEC_REMOTE_TERMINALS_AND_DURABLE_SESSIONS_2026_10_02.md`, which owns the connection model (§4), the helper (§6), durable sessions (§7), agent access (§8.2) and the open decisions (§11.2). This spec adds the place where the user sees and manages all of it. Pattern: `SPEC_ARMORY_ACCOUNTS_DELETE_AND_INLINE_DETAIL_2026_10_04.md` (the inline detail panel under a row, no modals).

## 1. Summary

AgentMux can open terminals and browse files on SSH hosts and WSL distributions. Today each pane picks its own connection, and nothing shows them all together:
- which machines you have;
- whether each is up;
- what AgentMux has put there;
- which agents may use it.

**Remotes** is a pane that does that: one list of every remote machine, its status and what is on it, with actions to open a terminal or browse files on it, and its settings in one place.

The connection picker inside each pane stays, as the way to choose where *that* pane runs. It gets the same names, colours and sections as the Remotes list, plus a "Manage remotes…" link to the pane.

Name chosen by the owner on 2026-10-05; "Outposts", "Uplink" and "Tower" were also considered. The internal and settings name stays `connections`.

## 2. What exists (`main` at `51e898fac`)

**Choosing a connection, per pane.**
- A terminal pane's header has a connection chip (`ConnectionButton`) that opens a typeahead picker (`ChangeConnectionBlockModal`, `conntypeahead.tsx`). It lists local, WSL distros (`WslListCommand`) and SSH names (`ConnListCommand`), and accepts a typed `user@host`.
- `ConnStatusOverlay` shows a dropped connection, with Reconnect and Disconnect.

**Hangar.**
- The sidebar has Places, Drives, WSL, **Remote** (`model.remotes()`) and Agents sections.
- The breadcrumb shows the host when the folder is on one.
- "Open terminal here" opens a terminal on the same connection.

**srv** (`backend/remote/`):

| Module | What it does |
|---|---|
| `ssh_config.rs` | Reads only the `Host` names from `~/.ssh/config`, following `Include` |
| `status.rs` | One status per connection, as `connchange` events, read with `GetAllConnStatus` (`connecting/connected/disconnected/error` plus the last error) |
| `helper_hosts.rs` | The hosts the helper has answered on (`remote-helper-hosts.json`) |
| `agent_access.rs` | Agents the user has always allowed on a host (`ssh-agent-access.json`) |
| `sessions.rs` | Durable sessions on a host, behind the pane menu's "Sessions on <host>…" |

**Commands.** `ConnList` returns names only: the ssh-config hosts plus any connection with a status this run. Also `ConnEnsure`, `ConnConnect`, `ConnDisconnect`, `ConnSessions`, `ConnSessionEnd` and `WslList`.

**Settings.** `settings.json` → `connections.<name>` (`ConnKeywords`): `display:order`, `display:hidden`, `term:theme`, `term:fontsize`, `term:fontfamily`, `term:durable`, `conn:shellpath`, `conn:ignoresshconfig`. It is edited only as JSON.

**Missing:**
- any view of all remotes at once;
- "recent", meaning hosts you used that are in neither ssh config nor settings;
- adding a host without editing `~/.ssh/config` by hand;
- per-host nickname or colour;
- the host's platform outside the Swarm;
- a way to see or revoke agent grants;
- the helper install question (`conn:helper`, decision 2 of the remote-terminals spec, not built);
- removing the helper from a host.

## 3. What comparable tools do

Researched on 2026-10-05: VS Code Remote-SSH and Remote Explorer, JetBrains Gateway, Zed, Wave, Warp, Termius, MobaXterm, Tabby, Royal TSX, and, for contrast, Windows Terminal and iTerm2 profiles.

| Practice | Seen in | Here |
|---|---|---|
| `~/.ssh/config` is the source of truth, read live, not imported once (imports drift and lose `ProxyCommand`) | VS Code, Zed, Wave, Gateway; Termius imports and drops `ProxyCommand` | §4.4 |
| An app file holds only per-host extras: nickname, order, hidden, theme | Wave `connections.json`, Zed `nickname` | §4.3 |
| WSL distros (and containers) found automatically, in the same list, grouped by type | VS Code Remote Explorer, Wave | §4.2 |
| Choose the target where the view is created, with typeahead (`user@host`) as quick connect | Wave (per block), Tabby Quick Connect | §4.7 |
| Always show which remote a view is on | VS Code status bar, Wave block header, iTerm2 badge | §4.8 |
| A colour or theme per host, so production looks different | Wave `term:theme`, Termius group themes | §4.3, §4.8 |
| Ask before installing anything on a host, remember per host, offer always and never, and a denylist | Warp, Wave (`conn:askbeforewshinstall`) | §4.9 |
| Say what the helper installs and where; fall back to a plain shell when the host is unsupported | Warp | §4.9 |
| Recent separate from saved | Gateway, VS Code | §4.2, §4.5 |
| Groups with inherited settings, once there are many hosts | Termius, Tabby, Royal TSX | later (§7) |

**Complaints to avoid**
- **Silent installs** of a server into the home directory of every host touched (VS Code, `vscode-remote-release#367`).
- **Installs that hang or fail with vague errors** (VS Code `#9758`, Zed `#37143`).
- **A remote wrapper that types into a session whose state is unknown** after wake (Warp `#15855`).
- **Host lists that vanish** when an unrelated part changes (VS Code `#8270`).
- **Two apps sharing one helper folder** (a Wave fork).
- **Writing to the wrong config entry** (Zed `#59676`).

## 4. Design

### 4.1 The pane

- **View type** `remotes`: label "Remotes", icon `server`, tooltip "Remote machines (SSH, WSL)". It is registered like Armory and Swarm.
- **One per window.** Opening it again focuses the existing one.
- **Opened from:**
  - the launcher and the command palette ("Open Remotes");
  - the connection picker's footer, "Manage remotes…";
  - the Remote heading in Hangar's sidebar;
  - a pane's connection chip menu, "Remote settings…", which opens the pane with that host's row expanded.

### 4.2 The list

A toolbar with **Add remote** (§4.4) and a filter box. Then sections, each hidden when empty:

| Section | Holds | Order |
|---|---|---|
| **Pinned** | remotes the user pinned | `display:order`, then name |
| **SSH hosts** | `Host` names from `~/.ssh/config` (as `ssh_config.rs` reads them), and remotes configured in `settings.json` | name |
| **Recent** | hosts used before that are in neither of the above, e.g. a typed `user@host` (§4.5) | most recent first; at most 20 |
| **WSL** | installed distributions (Windows only) | name |
| **Hidden** (collapsed) | anything with `display:hidden` | name |

A remote appears in exactly one section: Pinned wins, then Hidden, then the others.

**Each row shows:**
- **Name:** the nickname if set, with the real name beside it in muted text (`prod-db · db1.internal`), plus a colour swatch if set.
- **Platform:** `Linux` / `macOS` and the architecture, once known (§4.6).
- **Status:** a dot for connected, connecting, disconnected or error. Error shows its message on hover.
- **Helper:** installed (and its version), not installed, never (the user declined), or unsupported (no build for this platform).
- **Activity:** durable sessions running there (`2 sessions`), and agents with access (`1 agent`).

**Row actions** (buttons, and the right-click menu):
- **New terminal** opens a terminal pane on the remote.
- **Browse files** opens Hangar on the remote's home folder.
- **Sessions…** expands the row to its durable sessions (the "Sessions on <host>" list), with Reattach and End.
- **Connect** / **Disconnect**.
- **Pin** / **Unpin**, **Hide** / **Unhide**.
- **Forget** (Recent only).
- **Remove helper** (§4.9).
- **Settings** expands the inline detail panel (§4.3).

Clicking a row expands it; double-clicking opens a terminal.

### 4.3 Per-remote settings (the inline detail panel)

Every value is a key under `connections.<name>` in `settings.json`. Nothing about the SSH connection itself is stored there (§4.4).

| Setting | Key | New? |
|---|---|---|
| Nickname | `display:name` | new |
| Colour (6 swatches plus none) | `display:color` | new |
| Pinned | `display:pinned` | new |
| Order, hidden | `display:order`, `display:hidden` | exist |
| Terminal theme, font, size | `term:theme`, `term:fontfamily`, `term:fontsize` | exist |
| Keep sessions alive | `term:durable` | exists |
| Helper install: Ask / Always / Never | `conn:helper` | in the remote-terminals spec §4; first read here (§4.9) |
| Agents allowed here | `ssh-agent-access.json` (§4.10) | file exists |

For an SSH host, the panel also has:
- **Edit in ssh config**, which opens `~/.ssh/config` in the editor at that `Host` line;
- **Test connection** (§4.4).

AgentMux never rewrites an existing ssh-config entry, which avoids the "wrong entry" failure.

### 4.4 Adding a remote

**Add remote** expands a form at the top of the list:
- alias;
- host name or address;
- user;
- port;
- identity file (with a file picker);
- jump host.

Then **Test connection** and **Add**.

**Add appends a `Host` block to `~/.ssh/config`**, so every tool sees it, AgentMux included, and the file stays the one place hosts are defined:
- It refuses if the alias already exists.
- It keeps a backup of the file (`~/.ssh/config.agentmux-backup`) before the first write.
- It creates the file with mode `0600` (owner-only on Windows) if it is missing.
- It writes the block at the end, so an `Include` and earlier `Host *` defaults still apply as before.

**Test connection** runs `ssh -T <alias> -- true` through the askpass bridge (remote-terminals spec §5.3), so a password or new host key is asked in the approval window. It shows success, or ssh's own message.

A typed `user@host` in the picker still works without adding anything. It lands in Recent.

### 4.5 Recent

srv records each SSH connection that reaches `connected` in `remote-recent.json` under its config dir: name, first used, last used. That covers hosts typed into the picker and hosts an agent connected to.
- **Forget** removes an entry.
- An entry that becomes an ssh-config host or gets settings moves to that section automatically.
- After 20 entries, the oldest is dropped.

### 4.6 What srv tells the pane

A new command, `RemotesList`, returns one record per remote. `ConnList` keeps its shape for the picker and for agents.

```
{ name, kind: "ssh" | "wsl", sources: ["ssh_config" | "settings" | "recent"],
  status: { state, error }, platform: { os, arch } | null,
  helper: { state: "installed" | "absent" | "never" | "unsupported", version } ,
  sessions: n, agents: [agent ids], settings: { display:*, term:*, conn:helper } }
```

- **Platform** comes from the `uname -sm` the helper's probe already runs (`helper_install::probe_command`) and the ssh install path. srv keeps it per host beside the helper record (`remote-helper-hosts.json` gains `uname` and `seen_at`), so a host shows its platform from its first connection on, with or without the helper.
- **The list** is built from what already exists: `ssh_config::hosts`, the settings, `remote-recent.json`, `wsl::list`, `status::all`, `helper_hosts`, `agent_access`, and session counts.
- **Session counts** come from the last `ConnSessions` answer per host, refreshed when the row is expanded, so the list never opens an ssh connection by itself.
- **Updates** arrive as a `remoteschange` event whenever any of these change.

New commands besides `RemotesList`:
- `RemoteAdd` (§4.4);
- `RemoteTest` (§4.4);
- `RemoteForget` (§4.5);
- `RemoteHelperRemove` (§4.9);
- `RemoteAgentRevoke` (§4.10).

Settings changes go through the existing settings write.

### 4.7 The picker inside panes

The header picker stays the one way to choose where a pane runs. It becomes the same list in a smaller form:
- the same sections (Pinned, SSH hosts, Recent, WSL), with local at the top;
- the same nickname, colour swatch, platform and status dot as the Remotes rows;
- typing `user@host` or `wsl://distro` still connects directly;
- a footer, **Manage remotes…**, opens the Remotes pane.

It is used everywhere a pane can be remote:
- terminal;
- Hangar, from its toolbar as well as the sidebar, whose Remote section shows the pinned and SSH-host remotes with their colours;
- the editor, through a new "Open from remote…".

### 4.8 Which remote a pane is on

Any pane whose connection is not `local` shows, in its frame:
- the connection chip, with the nickname and the colour swatch;
- a 3 px stripe in the remote's colour along the top edge.

The tab of such a pane shows the swatch too. With no colour set, there is no stripe, only the chip. This is the P5 "per-connection colour accent" of the remote-terminals spec, built here.

### 4.9 The helper: ask first, remove any time

This builds decision 2 of the remote-terminals spec (§11.1), still outstanding there.

**When the helper is first needed on a host** (a durable pane, the host's files, or an agent's file access), and `conn:helper` is `ask` (the default), srv asks before uploading anything.

**The question appears in the host's approval window** (remote-terminals spec §5.3), which agents cannot reach or answer:

> Install AgentMux's helper on **db1.internal**?
> It enables file browsing and terminals that survive disconnects. About 500 KB, in `~/.agentmux-remote` on that machine; no root, removable any time from Remotes.
> **Install** · **Always for this host** · **Not now** · **Never for this host**

| Answer | Effect |
|---|---|
| Always / Never | Stored as `conn:helper` for that host |
| Install | Installs this time only |
| Not now, or Never | The pane falls back: a plain SSH terminal, or "File browsing on db1.internal needs AgentMux's helper", with an Install button and a link to the Remotes row |

**When an agent triggers it,** the question is always asked, whatever `conn:helper` says. Approving an agent's use of a host (§8.2) is not approving software installed there.

**Settings → Terminal → "Install the helper on new hosts"** sets the global default: Ask, Always or Never.

**Remove helper** (row action):
- stops the host's helper daemon, after the user confirms the sessions that would end;
- deletes `~/.agentmux-remote`;
- clears the host from `remote-helper-hosts.json`.

On a later need, the question is asked again unless the answer was Always.

### 4.10 Agents allowed on a remote

The detail panel lists the agents allowed on this host (`ssh-agent-access.json`), each with **Revoke**. Revoking takes effect for the agent's next action. A grant answered "once" isn't stored, so it isn't listed.

## 5. Phases

| Phase | Ships | Depends on |
|---|---|---|
| **R1** | The pane: `RemotesList` and `remoteschange`, sections, status, helper and session counts, New terminal and Browse files, pin, hide, nickname, colour; `remote-recent.json`; the platform record | — |
| **R2** | The picker as §4.7; frame chip and colour stripe (§4.8); Hangar's sidebar from `RemotesList` | R1 |
| **R3** | The install question (§4.9), the global default, Remove helper | R1 |
| **R4** | Add remote (ssh config write, backup, refuse duplicates), Test connection, Edit in ssh config | R1 |
| **R5** | Agents allowed: list and Revoke | R1 |

R3 should ship before the next release. The helper installs without asking until it does.

## 6. Tests

- **`RemotesList`:**
  - one record per remote, in exactly one section;
  - an ssh-config host that is also recent appears once, under SSH hosts;
  - WSL only on Windows;
  - opening the list runs no ssh.
- **Recent:** a typed `user@host` that connects is recorded; Forget removes it; 20 at most; it moves when added to ssh config.
- **Add remote:**
  - appends one `Host` block;
  - refuses a duplicate alias;
  - writes the backup once;
  - creates the file owner-only;
  - leaves every other line byte-identical (a golden-file test with `Include`, `Match` and `Host *`).
- **Install question:**
  - nothing is uploaded before an answer;
  - Always and Never are stored per host;
  - an agent-triggered first use asks even under Always;
  - the question goes to the approval window, not the main window;
  - Never gives the plain-shell fallback.
- **Remove helper:** stops the daemon, deletes the folder, clears the record; the next need asks again.
- **Revoke:** the agent's next action on that host is asked again.
- **Frame:** a remote pane with a colour shows the stripe and chip; local panes show neither.

## 7. Not in this spec

- **Groups or folders** with inherited settings, and tags. Worth it at tens of hosts (Termius, Tabby, Royal TSX); revisit when someone has that many.
- **A credential vault.** Keys and passwords stay with the user's ssh agent, ssh config and the askpass bridge.
- **Syncing remotes across machines.** `~/.ssh/config` is per machine; Recent and settings stay local.
- **Containers and Kubernetes** as remote kinds.
- **Windows as a remote host** (remote-terminals P6).
- **Hangar following the terminal's folder** (MobaXterm-style). A good next step once remote shells report their cwd (remote-terminals P5).

## 8. Decisions for the owner

1. **Add remote writes to `~/.ssh/config`** (recommended, so there is one place hosts are defined), or keeps AgentMux-only hosts in `settings.json`?
2. **WSL in Remotes** (recommended: one list for everything not local), or SSH only?
3. **One Remotes pane per window** (recommended), or any number?
4. **The colour stripe** on remote panes' frames (recommended), or the chip only?
5. **Order:** R1 and R3 first (the list, and asking before installing), then R2, R4 and R5?

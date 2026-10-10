# AgentMux widget samples

**Status:** living — the starter samples for the widget API (`docs/specs/SPEC_USER_WIDGETS_AND_WIDGET_API_2026_10_09.md`).

A widget adds your own kind of pane to AgentMux. Each folder here is a complete widget package you can install as it is, then copy and change. The full reference is `docs/specs/SPEC_USER_WIDGETS_AND_WIDGET_API_2026_10_09.md`; the SDK is `sdk/widget-sdk/`.

| Sample | Shows | Kind |
|---|---|---|
| `hello-sandboxed/` | The smallest widget: connect, set the title, count clicks in the pane's meta, a header action, follow the theme. Plain HTML, no build step. | sandboxed |
| `react-vite/` | The same, built with React and Vite. | sandboxed |
| `notes/` | Notes kept with `storage`, exported and imported with `files`, copied with `clipboard:write`. | sandboxed |
| `pr-dashboard/` | A repository's open pull requests from GitHub's API, with `net:https://api.github.com`; polls only while shown. | sandboxed |
| `ask-agent/` | Your agents and whether they're working (`agents:read`), and a message to one (`agents:send`). | sandboxed |
| `hello/` | A trusted widget: a Solid module that runs as part of AgentMux. | trusted |

## Quickstart: your first widget in five minutes

1. **Copy a sample.** Copy `hello-sandboxed/` somewhere of your own, say `~/my-widgets/acme.todo/`.
2. **Name it.** In its `widget.json`, set `id` to `<publisher>.<name>` (lowercase letters, digits and `-`, one dot, e.g. `acme.todo`). It must match the folder name. Set `name`, `description` and the pane's `label`.
3. **Install it.** In AgentMux: **Settings → Widgets → Install…**, pick your `widget.json`. AgentMux copies the folder into `~/.agentmux/widgets/acme.todo/` and asks you to approve it.
4. **Open it.** It's in the widget bar's **more** menu, and in any pane's **+** menu. Pin it from there.
5. **Edit and reload.** Edit the copy in `~/.agentmux/widgets/acme.todo/` (or your own copy, then install again with Replace). AgentMux sees the change and asks you to approve the new version; your open panes reload once you do.

## What a widget is

A folder with a `widget.json` and the widget's files:

```json
{
  "manifestVersion": 1,
  "id": "acme.todo",
  "name": "Todo",
  "version": "1.0.0",
  "kind": "sandboxed",
  "entry": "index.html",
  "permissions": ["storage"],
  "contributes": { "panes": [{ "name": "main", "label": "Todo" }] }
}
```

A **sandboxed** widget (the default) is a web page in a sandboxed frame. It can't reach AgentMux except through the SDK, and only for what it declared in `permissions` and you approved:

```js
import { connect } from "/agentmux/widget-sdk/v1.js";
const am = await connect();
await am.ui.setTitle("Todo");
await am.meta.set({ filter: "open" }); // this pane's own state
```

| Permission | Lets it |
|---|---|
| (none) | Use its pane: title, header actions, menu, toasts, theme, its own meta; open its own panes and links |
| `storage` | Keep its own data on this computer |
| `net:<origin>` | Connect to that origin (e.g. `net:https://api.github.com`) |
| `files` | Open files you pick, save files where you choose |
| `clipboard:write` | Copy text to your clipboard |
| `panes` | Open other kinds of panes |
| `agents:read` | See your agents' names and whether they're working |
| `agents:send` | Send messages to your agents |

A **trusted** widget (`"kind": "trusted"`) is a Solid ES module that runs as part of AgentMux, with full access. Use one only when a sandboxed widget can't do what you need, and only install trusted widgets from authors you trust.

## Rules worth knowing

- A widget runs only after you approve it, and asks again whenever any of its files change.
- A sandboxed widget loads code only from its own package (its scripts, styles and images), and reaches the network only through `am.net.fetch`, to origins it declared.
- If it navigates its frame to another page, AgentMux stops it.
- Packages are at most 50 MB and 2,000 files, with no symlinks.

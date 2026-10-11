# @agentmuxai/widget-sdk

The client for **sandboxed AgentMux widgets**. A widget's page runs in a sandboxed iframe inside an AgentMux pane; this SDK connects it to AgentMux and wraps the widget API (protocol 1).

The full reference is `docs/specs/SPEC_USER_WIDGETS_AND_WIDGET_API_2026_10_09.md` (§5 the package, §6 the bridge, §7 this SDK). Starter widgets are in `docs/examples/widgets/`.

## Use it without a build step

AgentMux serves the SDK itself, so a plain HTML widget imports it directly:

```html
<link rel="stylesheet" href="/agentmux/widget-sdk/am-widget.css">
<script type="module">
  import { connect } from "/agentmux/widget-sdk/v1.js";
  const am = await connect();
  await am.ui.setTitle("Hello");
</script>
```

## Use it with a bundler

```bash
npm install @agentmuxai/widget-sdk
```

```js
import { connect } from "@agentmuxai/widget-sdk";
```

That bundles a copy into your widget, which works with any AgentMux that speaks protocol 1, and gives you its types. Or keep importing it from `/agentmux/widget-sdk/v1.js`, the copy AgentMux serves, and tell the bundler to leave that import alone (Vite: `build.rollupOptions.external`; see `docs/examples/widgets/react-vite/`).

## The API

| | |
|---|---|
| `connect({ applyTheme?, timeoutMs? })` | The handshake. Resolves to the client; applies the app's theme as CSS variables and keeps it current. |
| `am.info` | The widget's id, version and pane; AgentMux's version; granted permissions; theme; this pane's meta. |
| `am.on(event, cb)` | `visibility`, `focus`, `theme`, `meta`, `action`, `command` (one of its palette commands or status items, below), `storage` (the widget's storage changed, in any of its panes), `dispose`. Returns an unsubscribe. |
| `am.meta.get()`, `am.meta.set(patch)` | This pane's own state, kept with the pane. |
| `am.ui.setTitle`, `setHeaderActions`, `setContextMenu`, `toast`, `openUrl` | The pane's chrome, drawn by AgentMux. |
| `am.ui.setStatusItem(id, { text, icon, tooltip, tone, hidden })` | Update one of the widget's status bar items while this pane is open. |
| `am.theme.get()` | The current theme. |
| `am.panes.open(view, meta?, split?)` | Open one of the widget's own panes (other views need `panes`). |
| `am.storage.*` | Per-widget data on this computer (`storage`). |
| `am.net.fetch(url, init?)` | HTTP to origins the widget declared (`net:<origin>`). |
| `am.files.pick()`, `am.files.save()` | Files the user picks (`files`). |
| `am.clipboard.writeText()` | `clipboard:write`. |
| `am.agents.list()`, `am.agents.send()` | `agents:read`, `agents:send`. |
| `useVisibility(am, { onActive, onDormant })` | Pause work while the pane is hidden. |
| `AgentMuxError` | `code`, `name` (`permission_denied`, …), `message`, `data`. |

A method a widget wasn't granted rejects with `AgentMuxError` named `permission_denied`; `err.data.permission` names what it needs.

## Commands and status bar items

A widget can add entries to the command palette and items to the status bar, declared in its `widget.json` (an AgentMux older than 0.59.19 ignores them):

```json
"contributes": {
  "panes": [{ "name": "main", "label": "Pull requests" }],
  "commands": [{ "id": "refresh", "title": "Refresh pull requests", "icon": "rotate-right" }],
  "statusItems": [{ "id": "count", "text": "PRs", "icon": "code-pull-request", "command": "refresh" }]
}
```

Running a command, from the palette or by clicking a status item that names it, focuses an open pane of the widget in the current tab, or opens one, and sends it the `command` event:

```js
am.on("command", ({ id }) => id === "refresh" && refresh());
await am.ui.setStatusItem("count", { text: `${prs.length} PRs`, tone: prs.length ? "info" : undefined });
```

A status item shows its `widget.json` look until a pane of the widget sets another, and again once that pane closes. Its tooltip always starts with the widget's name. Up to 20 commands and 4 status items per widget.

## Signing a widget

Signing lets AgentMux's install prompt say who published a widget, and warn when a later widget of the same publisher (the part of the id before the dot) is signed by another key. The package's `agentmux-widget` command does it with Node's own Ed25519:

```bash
npx -p @agentmuxai/widget-sdk agentmux-widget keygen ~/.agentmux-widget-key.json   # once; keep it private
npx -p @agentmuxai/widget-sdk agentmux-widget sign ./acme.notes --key ~/.agentmux-widget-key.json
npx -p @agentmuxai/widget-sdk agentmux-widget verify ./acme.notes
```

`sign` writes `widget.sig` next to `widget.json`; it covers every other file, so sign last: an edit afterwards makes AgentMux refuse the package until it is signed again. Publish your key's fingerprint (`K7Q2-MZ4D-PX3A-9TWE`) where users can check it. A signature names a key, not what the widget does: the permissions in the prompt are still all a sandboxed widget can do.

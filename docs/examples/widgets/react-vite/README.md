# Hello (React)

The sandboxed hello widget, built with React and Vite: a widget can use any web stack, since it runs in its own iframe and talks to AgentMux only through the SDK.

```bash
npm install
npm run build        # writes the package to dist/
```

Then in AgentMux: Settings → Widgets → Install…, pick `dist/widget.json`, and approve it.

`vite.config.js` keeps the SDK import (`/agentmux/widget-sdk/v1.js`) external, because AgentMux serves the SDK itself, and uses relative URLs (`base: "./"`), because the package is served from its own folder.

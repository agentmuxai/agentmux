# Fake web terminal

A page with real xterm.js in screen-reader mode and a tiny shell, for driving the
Browser tool's `terminal` action by hand or from a script
(`docs/specs/SPEC_BROWSER_WEB_TERMINAL_TYPING_2026_10_23.md` §13).

    node test/fixtures/fake-xterm/server.mjs [port]     # default 8765

The shell echoes each command as `ran: <line>`, turns Ctrl+C into `^C`, and
records what happened in page globals a test can read over CDP:

- `window.__ran`: the lines that were run, in order;
- `window.__keys`: every chunk xterm.js passed to `onData`;
- `window.__sigint`: how many Ctrl+C presses arrived;
- `window.__pasted`: text delivered by `paste` events.

Open it in a pane the agent owns, grant that pane the `terminal` power for
`http://127.0.0.1:<port>`, and call `terminal`.

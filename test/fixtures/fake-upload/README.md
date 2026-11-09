# Fake uploads

A page whose buttons open file choosers, for the Browser tool's `set_files` with
`chooser: true` on a grant that carries the `uploads` power
(`docs/specs/SPEC_BROWSER_WEB_TERMINAL_TYPING_2026_10_23.md` §6, §13).

    node test/fixtures/fake-upload/server.mjs [port]     # default 8767

Each button makes an `<input type=file>` and clicks it, so the chooser opens
during the agent's own click. `window.__got` holds one entry for each pick:
`{kind, files: [{name, size, text}], events: [{type, trusted}]}`; `window.__opens` has one
`{kind, active}` for each time the page asked for a chooser (`active`: the click had user activation).

| Button | The chooser |
|---|---|
| `#choose-one` | one file, anything |
| `#choose-many` | several files, `.txt` and `.pdf` only |
| `#choose-later` | one file, opened 300 ms after the click |
| `#choose-folder` | a folder: always cancelled |

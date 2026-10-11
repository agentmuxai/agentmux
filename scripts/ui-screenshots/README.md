# Docs screenshots and recordings

The images in agentmux-docs are taken by these scripts, so they can be
retaken when the UI changes. The design and the reasons behind it are in
`docs/specs/SPEC_UI_MANUAL_SCREENSHOT_TOOLING_2026_09_19.md` (§8 onward).

## When the UI has changed

1. **Build a portable** (`task package`) and start it isolated, with an empty
   data folder and a CDP port:

   ```bash
   AGENTMUX_HOME_OVERRIDE=<empty folder> AGENTMUX_CDP_PORT=9333 <portable>/agentmux.exe
   ```

   Strip any `AGENTMUX_*` variables the shell inherited from another
   instance. The data folder must not be your own `~/.agentmux`.

2. **Point it at the demo project** (once):

   ```bash
   node scripts/ui-screenshots/demo-env.mjs --home <that folder> --demo D:/demo/acme-web
   ```

3. **Retake everything and copy it into an agentmux-docs clone:**

   ```bash
   AGENTMUX_HOME_OVERRIDE=<that folder> node scripts/ui-screenshots/refresh.mjs \
       --port 9333 --docs <agentmux-docs clone> --ffmpeg <ffmpeg> \
       --redact "<that folder>=C:\Users\demo\.agentmux" --redact "<its parent folder's name>=demo" \
       --redact "<the build's channel>=stable"
   ```

   This runs every suite (widgets, chrome, agent, memory) and every
   recording, then replaces each image the docs already use. Nothing is
   added to the docs: images they don't use yet are listed.

4. **Review before you commit.** Open every changed image (`git diff --stat`
   in the docs clone, then look at each one) for correctness and for anything
   from your machine. Then commit with a changeset and open a PR.

## Pieces

| Script | What it does |
|---|---|
| `capture.mjs --suite <name>` | Takes one suite's screenshots: `widgets`, `chrome`, `agent`, `memory` (and `manual`, the first run). |
| `record.mjs` | Records the clips in `recordings.mjs` as GIF (and `--formats webp,mp4`). Needs ffmpeg. |
| `publish.mjs --docs <clone> --from <run>...` | Copies a run's images over the docs' files of the same name. `--dry-run` shows what would change. |
| `refresh.mjs` | All of the above, in order. |
| `demo-env.mjs` | The made-up `acme-web` project the widgets open. |
| `install-fake-claude.mjs` | The scripted stand-in for Claude Code the agent suite uses (no account, model or network). |

Retake one shot or clip with `--only <id>` on `capture.mjs` or `record.mjs`.

## Adding a shot or a clip

- **A shot** goes in its suite file (`chrome-shots.mjs`, `agent-shots.mjs`,
  …): an `id`, what to do first (`prep`), what must be on screen (`verify`),
  what to crop to (`selector`), and what to undo (`cleanup`).
- **A clip** goes in `recordings.mjs`: `prep` (not recorded) returns the crop,
  and `run` is recorded. Move the drawn cursor with `cursor.clickSelector()`.
- **Name the docs image after the id** (`<id>.png`, `<id>-<size>.png` or
  `<id>.gif`), so `publish.mjs` can find it next time.

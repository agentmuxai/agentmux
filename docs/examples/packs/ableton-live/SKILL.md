---
name: ableton-live
description: Use when you need to make or change music in Ableton Live (Live.exe on Windows, Ableton Live on macOS): tracks, MIDI clips, devices, tempo, playback. Covers the Remote Script and MCP route first, then Live's keyboard shortcuts, then Desktop clicks, with a check after each step.
---

# Ableton Live

**Status:** draft — unverified: written from Ableton's manual and the AbletonMCP README, not yet run on any Live version.

Pack: `ableton-live` v0.1, unverified. Executables: `Live.exe` (Windows), the `Ableton Live` app (macOS). Written from Ableton's manual and the AbletonMCP project's README; no step has been run by an agent yet (see Proof).

## The ladder: use the first route that works

1. **The Remote Script bridge (AbletonMCP).** A Remote Script runs inside Live and opens a local socket; an MCP server (`uvx ableton-mcp`) exposes it as tools: create and change tracks, MIDI and audio clips, load instruments and effects, tempo, playback, session info. This is far better than clicking.
   - Needs: Live 10 or newer, Python 3.8 or newer, the `uv` package manager installed first.
   - Install the script with `uvx --from ableton-mcp ableton-mcp-install-script`. It lands in `Documents\Ableton\User Library\Remote Scripts\AbletonMCP\` (Windows) or `~/Music/Ableton/User Library/Remote Scripts/AbletonMCP/` (macOS).
   - In Live: Settings (Preferences) > Link, Tempo & MIDI > **Control Surface** = `AbletonMCP`, Input and Output both `None`.
   - Run **one** MCP server instance only (not in two clients at once).
   - The server is installed from the MCP tab, with the person's approval. A pack never installs it, and some variants of this project run Python you send them inside Live: treat the bridge as full control of the Live process.
2. **Live's own keyboard shortcuts** through `Desktop` (below), for what the bridge does not cover.
3. **`Desktop` clicks**, last, and only in Live's own windows. Plug-in windows are drawn by the plug-in and are pixels only.

## Shortcuts (Live 12 manual; Windows / macOS)

Switch Session and Arrangement `Tab` · Insert audio track `Ctrl+T` / `Cmd+T` · Insert MIDI track `Ctrl+Shift+T` / `Cmd+Shift+T` · Insert MIDI clip `Ctrl+Shift+M` / `Cmd+Shift+M` · Play and stop `Space` · Record `F9` · Arrangement record `Shift+F9` · Undo `Ctrl+Z` / `Cmd+Z` · Redo `Ctrl+Y` / `Cmd+Shift+Z` · Save `Ctrl+S` / `Cmd+S` · Save As `Ctrl+Shift+S` / `Cmd+Shift+S` · Browser toggle `Ctrl+Alt+B` / `Cmd+Option+B` · Detail view toggle `Ctrl+Alt+3` / `Cmd+Option+3` · Duplicate `Ctrl+D` / `Cmd+D` · Rename `Ctrl+R` / `Cmd+R` · Loop toggle `Ctrl+L` / `Cmd+L` · Launch clip `Enter` · Delete `Delete`.

## Pitfalls

- **Save the set before you experiment.** The bridge changes the open set, and Live's undo is the only way back. Ask the person to save, or save a copy first (`Ctrl+Shift+S`).
- **Stepped requests.** Create the track, check it exists, then add the clip, then the notes. One big request that fails midway leaves a half-built set.
- **Default devices only.** The bridge relies on Live's own instruments and effects; third-party plug-ins are not reliably loadable or controllable.
- **Plug-in UIs are custom-drawn.** `Desktop` sees pixels there; prefer the device's parameters through the bridge.
- **Audio is the person's.** Do not start playback or recording on a set without saying so first; playing sound in an occupied room is not a harmless test. Mute the master or ask.
- **A Control Surface that is not selected looks like a dead bridge.** If the server cannot connect, check Preferences > Link, Tempo & MIDI before anything else.
- Never type a licence key or account password; ask the person.

## Verify after each step

| Step | Cue that it worked |
|---|---|
| Bridge connected | the server's session-info tool answers with the set's tempo and tracks |
| Track created | the track count rose by one; Session view shows its name |
| Clip created | the clip's length and notes read back through the bridge match what you sent |
| Playback | the transport shows playing; stop it with `Space` when done |
| Saved | the set's file time changed; the title bar shows no unsaved mark |

## Proof

`test.json` is the pack's test. Verified on: none yet.

# Packs (examples)

**Status:** active — Stage 1 of the packs spec; the Blender pack is verified, the other four are not.

Per-application knowledge for agents that drive desktop apps. A pack is versioned: `version`, `changelog`, `app.versions` and `verified` records that name the pack version (spec §10); a line that differs between app versions starts with a range tag such as `[v >=21]`. See `docs/specs/SPEC_PACKS_2026_11_05.md` for the format and `docs/reports/REPORT_COMPUTER_USE_STATE_AND_APP_SKILL_PACKS_2026_11_05.md` for why.

| Folder | App | Best route | State |
|---|---|---|---|
| `write-a-pack` | the guide: how to write a pack | n/a | a skill |
| `word` | Microsoft Word for Windows | COM (`Word.Application`) | unverified |
| `ableton-live` | Ableton Live | Remote Script and MCP | unverified |
| `wavelab` | Steinberg WaveLab | ActiveX/COM scripting | unverified |
| `excel` | Microsoft Excel for Windows | COM (`Excel.Application`), openpyxl for a closed file | unverified |
| `blender` | Blender (Windows, macOS, Linux) | headless Python, `blender --background --python` | verified on Windows, Blender 5.2.3 (`pack_test.py`, `check_png.py`) |

## Stage 1: use a pack today

Stage 1 packs are plain skills: everything an agent needs is in each `SKILL.md`; `pack.json` and `test.json` are the manifest and the test that later stages read. To use one now, import its folder in Memory > Skills (import from a local folder), then bind it to an agent that will meet the app (a pack is not given to every agent). A pack grants nothing: the `Desktop` tool still asks the person, per app.

## Verifying

Run a pack's `test.json` on a machine that has the app and `Desktop` control switched on, and append the result to `pack.json` `verified`. Until then a pack is unverified.

---
name: word
description: Use when you need to create, read or edit a Microsoft Word document, or drive Word (WINWORD.EXE) on Windows. Covers the COM route first, then shortcuts, then Desktop clicks, with a check after each step.
---

# Word (Microsoft Word for Windows)

**Status:** draft — unverified: written from Microsoft's documentation, not yet run on any Word version.

Pack: `word` v0.1, unverified. Executable: `WINWORD.EXE`. Written from Microsoft's documentation; no step below has been run by an agent yet (see Proof).

## The ladder: use the first route that works

1. **The file, when Word is not needed.** A `.docx` is a zip of XML. Create or edit it with a library (python-docx) or the `docx` skill, with Word closed. No Word window, nothing to grant.
2. **Word's own object model (COM), for a document that must be open in Word.** ProgID `Word.Application`. From PowerShell:
   - `$w = New-Object -ComObject Word.Application` (or `[Runtime.InteropServices.Marshal]::GetActiveObject("Word.Application")` to attach to the one the person has open)
   - `$d = $w.Documents.Add()`; `$w.Selection.TypeText("text")`
   - `$d.SaveAs2([ref]"C:\full\path\name.docx", [ref]12)` (FileFormat 12 is `wdFormatXMLDocument`, `.docx`; 17 is `wdFormatPDF`). Pass full paths. PowerShell wants `[ref]` for these arguments.
   - `$d.Close(); $w.Quit()` when you started it. Release it, or a hidden `WINWORD.EXE` stays running.
   - A Word MCP server (WordMcp.McpServer, word-mcp-live) wraps the same COM calls. Use one if it is installed (Packs tab shows the plug-in state); installing one is the person's approval, never yours.
3. **Shortcuts and menus** through `Desktop` (below).
4. **`Desktop` clicks**, last: screenshot, one action, screenshot again.

## Shortcuts (Word for Windows, from Microsoft's shortcut list)

New `Ctrl+N` · Open `Ctrl+O` · Save `Ctrl+S` · Save As `F12` · Close `Ctrl+W` · Undo `Ctrl+Z` · Redo `Ctrl+Y` · Find and replace `Ctrl+H` · Select all `Ctrl+A` · Start `Ctrl+Home` · End `Ctrl+End` · Bold `Ctrl+B` · Italic `Ctrl+I` · Heading 1 `Ctrl+Alt+1` · Heading 2 `Ctrl+Alt+2` · Print `Ctrl+P` · Ribbon key tips `Alt` · Tell Me search `Alt+Q`.

For anything with no chord: press `Alt+Q`, type the command name, press `Enter`. It beats hunting the ribbon by pixel.

## Pitfalls

- **A modal dialog blocks COM.** Calls fail with "call was rejected by callee" while a dialog (Save As, a recovery prompt, a licence notice) is open. Look at the window, answer the dialog by keyboard, then retry.
- **Protected View** (a downloaded file) opens read-only; editing needs the person to click Enable Editing. Say so; do not hunt for the button as a workaround.
- **AutoSave and OneDrive** change the save flow for a cloud document: `Ctrl+S` may save to the cloud with no dialog. Save to a local path for tests.
- **File names:** keep the extension in the name you pass; an existing file raises an overwrite prompt that stops COM.
- **One Word at a time.** A second `New-Object` may attach to the person's session. Check `$w.Documents.Count` before you close or quit anything: it may not be yours.
- Never type a password or a licence key; ask the person.
- `Desktop` needs the person's per-app grant for Word like any other app. A pack never grants it.

## Verify after each step

| Step | Cue that it worked |
|---|---|
| Document created | `$w.Documents.Count` rose by one; the window title is `Document1 - Word` |
| Text typed | `$d.Content.Text` contains the sentence |
| Saved | the file exists, size above 0, the title bar shows the file name |
| `.docx` content | the zip holds `word/document.xml` and it contains the sentence |
| Closed | the process list no longer shows the `WINWORD.EXE` you started |

## Proof

`test.json` is the pack's test. Verified on: none yet. A pack is "verified" only for an app version where that test passed and the result was recorded.

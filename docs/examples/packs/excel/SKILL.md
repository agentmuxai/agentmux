---
name: excel
description: Use when you need to create, read or edit a Microsoft Excel workbook, or drive Excel (EXCEL.EXE) on Windows: cells, formulas, sheets, saving. Covers COM first for a workbook open in Excel, a library for a closed .xlsx, then shortcuts, then Desktop clicks, with a check after each step.
---

# Excel (Microsoft Excel for Windows)

**Status:** draft — unverified: written from Microsoft's and the libraries' documentation, not yet run on any Excel version.

Pack: `excel` v0.1.0, unverified. Executable: `EXCEL.EXE`. Written from Microsoft's documentation and the library docs; no step below has been run by an agent yet (see Proof).

## The ladder: use the first route that works

1. **The file, when Excel need not be open.** A `.xlsx` is a zip of XML. Use a library with Excel closed: **openpyxl** (read and write; it does **not calculate** formulas) or **XlsxWriter** (write only; `write_formula(row, col, formula, format, value)` takes the result as its last argument, so a reader that does not recalculate still sees a value). With openpyxl, `load_workbook(path, data_only=True)` returns the value Excel last saved for a formula cell, and `None` for a formula Excel never calculated: a file written by openpyxl has no results until Excel opens and saves it.
2. **Excel's object model (COM), for a workbook that must be open or a formula that must be calculated.** ProgID `Excel.Application`. From PowerShell:
   - `$xl = New-Object -ComObject Excel.Application; $xl.DisplayAlerts = $false`
   - `$wb = $xl.Workbooks.Add(); $ws = $wb.Worksheets.Item(1)`
   - `$ws.Range("A1").Value2 = 1` (also `A2`, `A3`); `$ws.Range("B1").Formula = "=SUM(A1:A3)"` (`.Formula` takes English function names whatever the person's language; `.FormulaLocal` takes the local ones)
   - `$ws.Range("B1").Value2` reads the calculated result
   - `$wb.SaveAs("C:\full\path\book.xlsx", 51)` (51 is `xlOpenXMLWorkbook`, `.xlsx`); pass a full path
   - `$wb.Close($false); $xl.Quit()` when you started it
   - A COM-based Excel MCP server wraps the same calls; use one if installed. Installing one is the person's approval, never yours.
3. **Shortcuts and menus** through `Desktop` (below).
4. **`Desktop` clicks**, last: screenshot, one action, screenshot again. Use zoom on a small value.

## Shortcuts (Excel for Windows, from Microsoft's shortcut list)

New `Ctrl+N` · Open `Ctrl+O` · Save `Ctrl+S` · Save As `F12` · Close `Ctrl+W` · Undo `Ctrl+Z` · Redo `Ctrl+Y` · Edit the cell `F2` · Go to `Ctrl+G` or `F5` · Select all `Ctrl+A` · Jump to the edge of the data `Ctrl+Arrow` · Select to the last used cell `Ctrl+Shift+End` · Confirm `Enter` · Fill down `Ctrl+D` · Fill right `Ctrl+R` · AutoSum `Alt+=` · Insert function `Shift+F3` · Calculate the sheet `Shift+F9` · Calculate everything `Ctrl+Alt+F9` · New sheet `Shift+F11` · Next sheet `Ctrl+Page Down` · Previous sheet `Ctrl+Page Up` · Tell Me search `Alt+Q` · Ribbon key tips `Alt`.

To reach a cell fast: `Ctrl+G`, type the address (`B1`), `Enter`.

## Pitfalls

- **A file written by a library has no formula results** until Excel calculates and saves it (route 1). If a person or a test needs the value, write it with COM, or write the value alongside with XlsxWriter, or open and save once in Excel.
- **Calculation mode.** If the workbook is on Manual, `Value2` of a formula may be stale: call `$xl.Calculate()` first, or press `Ctrl+Alt+F9`.
- **Modal dialogs block COM** ("call was rejected by callee"): a Save As, a recovery prompt, a licence notice. Look at the window, answer it by keyboard, then retry. Setting `DisplayAlerts = $false` stops overwrite prompts, but also hides what Excel would have asked: say so in your answer.
- **Protected View** opens a downloaded file read-only; editing needs the person to click Enable Editing.
- **Locale.** A decimal comma or a local list separator changes what typed text means; use `Value2` with numbers, not strings, and `.Formula` with English names.
- **One Excel at a time.** A second `New-Object` may attach to the person's session. Before you quit, check `$xl.Workbooks.Count`: the other workbooks may not be yours. Never quit an Excel you did not start.
- **Do not leave `EXCEL.EXE` running.** An unreleased COM object keeps a hidden process; close and quit what you opened.
- Macros (`.xlsm`) run code: never enable them for a file you did not write. Never type a password or a licence key; ask the person.
- `Desktop` needs the person's per-app grant for Excel like any other app. A pack never grants it.

## Verify after each step

| Step | Cue that it worked |
|---|---|
| Workbook created | `$xl.Workbooks.Count` rose by one; the window title is `Book1 - Excel` |
| Values written | `$ws.Range("A1:A3").Value2` reads back what you wrote |
| Formula | `$ws.Range("B1").Formula` is the formula text and `.Value2` is the result (6 for `=SUM(A1:A3)` over 1, 2, 3) |
| Saved | the file exists, size above 0; openpyxl `data_only=True` on it returns the same result |
| Closed | the `EXCEL.EXE` you started is gone |

## Proof

`test.json` is the pack's test. Verified on: none yet.

# Editor: file tree icons and keyboard, which region has focus, click zoom, and files another program is using

**Status:** active. Requested by the owner on 2026-10-16, decisions confirmed the same day (§10). Being built in three PRs (§9, Order); it follows the editor revamp's phase 1 (#5016), which owns the image viewer this changes.

## 1. What the owner asked for

1. The file tree's icons say what each file is, in more colours, and its file names are VS Code's grey rather than white.
2. The file that is open in the editor is marked in the tree. Today nothing shows.
3. The tree is fully keyboard driven: after picking a file, ↑ and ↓ move to the previous and next file, following the established practice for trees.
4. It is always clear whether the tree or the editor body has the keyboard.
5. Image zoom by clicking: each click zooms in a little, five clicks reach the deepest zoom, and Ctrl+click zooms back out a step at a time.
6. A file another program is using opens read-only, with a note saying why. Today such a file doesn't load at all.

The editor body's own keyboard handling (CodeMirror) is good and stays as it is.

## 2. How it works today (read from the code, 2026-10-16)

- **Icons** (`file-tree.tsx`, `iconForEntry`): seven generic Font Awesome icons (`file-code` for every language, `file-lines`, `file-image`, `file-pdf`, `file-zipper`, `folder`, `file`). They are all the text colour at 80% opacity. The open file's icon is replaced by `circle-dot`, so its type is lost.
- **Text colour:** the tree inherits `--main-text-color`, which is `#f7f7f7` on the default dark theme: near white.
- **Open file:** a row is `file-tree-row--active` when `props.path === props.activeFilePath`, with a 15% accent tint. The owner sees nothing, and the likely cause is the comparison itself. The tree builds paths with `joinPath`, which takes whichever separator the root path has, while `activeFilePath` is the tab's `filePath`, written however the file was opened (Files pane, an agent, a drop, the command line). On Windows one side can have `\` and the other `/`, or a different drive-letter case, and then no row ever matches. The tree also doesn't expand to or scroll to the open file, so even a matching row is often out of view. The code settles the first on Windows: the tab store writes every tab's path through `canonicalizePath` (`editor-pane-state-store.ts`: `/` separators, a lower-case drive letter), so a tab's `c:/Users/a/x.ts` never equals the tree's `C:\Users\a\x.ts`, whichever way the file was opened, a click in the tree included. On macOS and Linux the two match, and only the missing reveal applies.
- **Keyboard:** the tree is one focusable `div` (`tabIndex=0`) whose only key is F2 (rename). There are no tree roles, and arrows do nothing. `SPEC_EDITOR_FILE_TREE_2026-05-26.md`'s status line says keyboard navigation is in; it isn't, and that line gets corrected with this work.
- **Clicks:** a click opens a preview tab; a double-click opens a tab that stays (`docs/keybindings.md`, Editor).
- **Focus:** nothing shows which of the tree and the editor body has the keyboard, apart from CodeMirror's caret.
- **Image zoom** (`viewers/image/image-zoom.ts`, from #5016):
  - the wheel zooms about the pointer, about 20% per notch;
  - a drag pans;
  - a double-click switches between fit and actual size;
  - the keys are `+` `−` `0` `1` and the arrows.

  Zoom runs from fit (scale 1) to `maxScale(box) = max(4, 32 × natural width / fit width)`.

## 3. Tree look

### 3.1 File names in VS Code's grey

| Theme family | Colour | VS Code's source |
|---|---|---|
| Dark | `#CCCCCC` | `sideBar.foreground` in `dark_modern.json` |
| Light | `#3B3B3B` | `sideBar.foreground` in `light_modern.json` |

A new token, `--file-tree-text-color`, holds these values: the dark value in `:root` and the light value in the `[data-theme-polarity="light"]` block (`theme.scss`). The tree's rows use it. The open file keeps its accent colour (§4).

### 3.2 Icons by file type, in VS Code's Seti colours

A new module, `file-icons.ts`, maps a file to a Font Awesome icon and a tone. It tries the whole file name first (`package.json`, `Dockerfile`, `.gitignore`, `Cargo.toml`, `README.md`, `LICENSE`, `.env*`, `Makefile`), then the extension. The language's own mark is used wherever Font Awesome Free 6.7.2, which the app ships, has one:
- brands: Python, Rust, Go, JavaScript, React (`.tsx`/`.jsx`), HTML5, CSS3, Sass, Markdown, Java, PHP, Swift, Docker, Git, npm;
- otherwise a fitting solid icon: `c`, `hashtag` (C#), `gem` (Ruby), `terminal` (shell scripts), `gear` (YAML/TOML/INI), `database` (SQL/SQLite), `file-csv`, `file-excel`, `file-word`, `file-powerpoint`, `file-pdf`, `file-image`, `file-audio`, `file-video`, `file-zipper`, `lock`, `key`, `certificate`, `font`, `cube` (binaries, WebAssembly), `book` (notebooks);
- anything else: `file`.

**Tones** are the Seti palette VS Code's default file icons use (`jesseweed/seti-ui`, `styles/ui-variables.less`):

| Tone | Dark | Used for, mainly |
|---|---|---|
| blue | `#519aba` | TypeScript, Python, Go, CSS, Markdown, Word, C |
| green | `#8dc149` | shell scripts, CSV and spreadsheets, Vue |
| yellow | `#cbcb41` | JavaScript, JSON, archives, keys, licences, SVG |
| orange | `#e37933` | Rust, HTML, Swift, Git, XML, notebooks, Makefiles |
| red | `#cc3e44` | Java, Ruby, PDF, fonts, npm |
| purple | `#a074c4` | images, YAML, PHP, headers, WebAssembly |
| pink | `#f55385` | Sass, SQL and databases, audio, video |
| steel | `#7494a3` | folders, TOML/INI, text |
| grey | `#6d8086` | lock files, logs, binaries, unknown files |

Each tone is a token, `--file-icon-<tone>`. On light themes, yellow becomes Seti's `#a2852f`, since `#cbcb41` is unreadable on white; the other tones keep their values. The icon is drawn at full opacity.

### 3.3 Folders

`folder` when closed and `folder-open` when expanded, in steel. The chevron stays.

## 4. The open file

1. **Matching:** the row matches when the paths are equal after normalising: `\` to `/`, no trailing separator, and on Windows a case-insensitive comparison. One helper does this, used by both the tree and its test.
2. **Look:** a 2 px accent bar on the row's left edge, the existing accent tint and accent text. The icon keeps the file's own icon; `circle-dot` goes. The bar keeps the open file visible even when another row has the keyboard (§5).
3. **Reveal**, like VS Code's `explorer.autoReveal`: when the open file changes (a tab switch, an agent opening a file, following the folder), the tree expands its parent folders under the root that holds it and scrolls the row into view only if it is off screen. A file outside every root stays unrevealed.

## 5. Keyboard: the WAI-ARIA tree pattern

The practice to follow is the W3C ARIA Authoring Practices' tree view pattern, the same one VS Code's explorer follows. It covers roles, a single tab stop, and arrow keys that move through the visible rows.

### 5.1 Structure

- The tree has `role="tree"`, each row `role="treeitem"`, and each folder's children `role="group"`. Folders carry `aria-expanded`; the selected row carries `aria-selected="true"`. `aria-level`, `aria-setsize` and `aria-posinset` are set, since rows load lazily.
- **One tab stop, using roving `tabindex`:** the selected row has `tabindex="0"` and every other row `-1`. Tab from outside lands on the selected row, or on the first row if none is selected; Tab again leaves the tree.
- **Two separate ideas:** the *selected* row is where the keyboard is. The *open* row is the file in the editor (§4). They are usually the same row, but ↑/↓ moves the selection first.

### 5.2 Keys, when the tree has focus

| Key | Does |
|---|---|
| ↓ / ↑ | Select the next or previous visible row, and preview it if it's a file (§5.3). |
| → | On a closed folder, expand it. On an open folder, select its first child. On a file, nothing. |
| ← | On an open folder, collapse it. Otherwise, select the parent folder. |
| Home / End | Select the first or last visible row. |
| Page Down / Page Up | Move by a screenful of rows. |
| Enter | On a file, open it in a tab that stays and move focus to the editor body. On a folder, expand or collapse it. |
| Space | On a file, keep its preview tab (the same as a double-click) and stay in the tree. On a folder, expand or collapse it. |
| Letters | Type-ahead: select the next visible row whose name starts with what was typed in the last 500 ms. |
| F2 | Rename (as today). |
| Escape | Move focus to the editor body, without changing anything. |

### 5.3 ↑/↓ previews the file

The owner asked that ↓ after picking a file "selects the next file". That should show the file, not only move a highlight, as single-clicking already does. So moving onto a file opens it in the preview tab, which replaces the previous preview, the same way a click does today. It uses a short debounce (100 ms), so holding ↓ doesn't load every file it passes, and focus stays in the tree. Folders are selected without opening anything.

### 5.4 Moving between the tree and the editor body

- **Ctrl+Shift+E** (⇧⌘E on macOS) moves the keyboard from the editor body to the tree, showing the tree if it is hidden, and from the tree back to the editor body. It is VS Code's own "focus the Explorer" key, and it isn't a default here: it only appears as an example of a custom binding in `docs/keybindings.md`. F6 was the other candidate, but it is already the app's Next pane key.
- Enter or Escape in the tree also moves to the editor body (§5.2).
- Clicking in either region moves the keyboard there.

## 6. Which region has the keyboard

The display follows VS Code's active and inactive list selection, plus one more cue, because the owner wants it unmistakable:

- **Selected row, tree focused:** strong accent fill and a 1 px focus outline (`@include mixins.focus-ring`).
- **Selected row, tree not focused:** a neutral grey fill (VS Code's inactive selection), so it stays visible but clearly isn't live.
- **Open file:** the accent bar from §4 shows in both cases.
- **The focused region,** tree or editor body, gets a 2 px accent line along its top edge. The other region's line is transparent, so nothing shifts. This works with every theme, needs no colour decoding, and can't be confused with selection.

## 7. Image zoom by clicking

### 7.1 Behaviour

- **A click** (a press and release that moves less than 4 px) zooms in one step about the pointer. A press that moves 4 px or more is a pan, as today.
- **The steps:** five geometric steps from fit to the deepest zoom. Step *k*, for *k* = 0 to 5, is scale `maxScale^(k/5)`, so step 0 is fit and step 5 is `maxScale`. Each click goes to the next step above the current scale, so after a wheel zoom the next click lands on the next step rather than a fixed jump. At step 5 a click does nothing more.
- **Zooming out:**
  - Ctrl+click on Windows and Linux;
  - ⌥+click on macOS, where Ctrl+click is a right-click;
  - Alt+click on every platform as well.

  Each goes to the next step below the current scale and stops at fit.
- **Double-click no longer toggles fit and actual size.** Two quick clicks would otherwise zoom in twice and then jump. Fit stays on `0` and the Fit button, and actual size on `1` and its button. The `docs/keybindings.md` row changes to match.
- **Cursor:** `zoom-in` over the image, `zoom-out` while the zoom-out modifier is held, and `grab` at the deepest zoom, where only panning is left.
- The wheel, the keys and the background cycle stay as they are.

### 7.2 Step size

The steps are evenly spaced on a log scale, so each click looks like the same amount of zoom. How big a step is depends on the image: for a picture the size of the pane, `maxScale` is 32 and a step is 2×; for a large photo shown small, `maxScale` is larger and the steps are larger. If the owner finds 2× too coarse ("a little increment"), `maxScale` can be capped for click zoom only, for example at 16× per step 1.74×. That's a one-line change in `clickSteps()`.

## 8. Files another program is using

### 8.1 Why such a file doesn't load today

`readeditorfile` (`crates/srv/src/server/editor_handlers.rs`) reads with `std::fs::read`, and phase 1's probe (`editor_probe.rs`) with `std::fs::File::open`. On Windows, Rust opens with every sharing flag (`FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE`), so a file another program has open, even for writing, reads fine as long as that program allows others to read. Such a file fails in three cases:

| Case | Windows error | Can we show the contents? |
|---|---|---|
| The other program opened it without allowing reads (an exclusive open) | 32, `ERROR_SHARING_VIOLATION` | No. No normal API reads it until that program closes it. Backup semantics and shadow copies need administrator rights, which the editor doesn't have and shouldn't ask for. |
| The other program locked a byte range in it (`LockFileEx`; Office documents, some databases) | 33, `ERROR_LOCK_VIOLATION`, on the read, not the open | Partly: everything outside the locked ranges. |
| The other program allows reads but not writes | none on read; a save fails with 32 | Yes, fully. Only saving is the problem. |

Today, in every case, the error ends up as a failed read string (`readeditorfile: ... (os error 32)`). The tab shows nothing useful, and in the third case a save fails late, after the user has typed. Which case the owner hit isn't known yet; the dev-build check in §8.4 covers all three.

### 8.2 What the editor does instead

1. **srv classifies the failure** and returns a structured `locked` result instead of an error string: which case (§8.1), and the programs holding the file. The programs come from the Windows Restart Manager (`RmStartSession`, `RmRegisterResources`, `RmGetList`), the same API Explorer uses for "The action can't be completed because the file is open in ...". It needs no elevation and gives each program's name and PID. If the lookup fails or finds nothing, the note says "another program" without a name.
2. **Readable, write-blocked** (the third case): when opening a file for editing, srv also asks whether a write would be allowed, by opening for write with full sharing and closing at once. If the answer is a sharing violation, the file opens **read-only**: the editor shows the contents, saving is disabled, and a note sits above the text:

   > Read-only: Excel (PID 1234) has this file open. Close it there to edit. [Check again]

   *Check again* re-runs the check. The editor also re-checks on its own when the window regains focus. Once the file is free, the note goes and the tab becomes editable, keeping anything already loaded.
3. **Byte-range locked** (the second case): srv reads the unlocked ranges and fills locked ones with zeros. The file opens **read-only** in the viewer phase 1 picks for it, mostly the info card or hex for such files, with a note:

   > Read-only, incomplete: Outlook (PID 4321) has locked parts of this file; those parts show as zeros.

   A text viewer is never offered for such a file, since the zeros would read as real content.
4. **Read-blocked** (the first case): there are no contents to show, so the tab shows the info card (name, size, dates) with the note:

   > Can't open: Excel (PID 1234) is using this file and doesn't let other programs read it. Close it there, then [Try again].

   It retries on window focus as in step 2.
5. **Saving a file that became write-blocked after it opened:** the save fails with the same note and the program's name, the edits stay in the tab, and the tab becomes read-only until *Check again* succeeds.
6. **Other platforms:** macOS and Linux locks are advisory and don't block reads or writes, so this only changes Windows. A plain permission error (`EACCES`, `ERROR_ACCESS_DENIED` 5) keeps today's existing read-only and permission notes, without the program lookup.

### 8.3 Implementation

| File | Change |
|---|---|
| `crates/srv/src/backend/file_lock.rs` (new, Windows only; a stub elsewhere) | `classify(path, io::Error) -> Option<LockInfo>`; `holders(path) -> Vec<Holder { name, pid }>` through the Restart Manager; `write_blocked(path) -> bool`; `read_unlocked(path) -> (bytes, locked_ranges)` |
| `crates/srv/src/server/editor_handlers.rs` | `readeditorfile` returns `read_only_reason: Option<ReadOnlyReason>` alongside the existing `read_only`; `writeeditorfile` maps 32 and 33 the same way |
| `crates/srv/src/backend/editor_probe.rs` | A `locked` result for a probe that hits 32 or 33, so phase 1's viewer choice can still decide what to show |
| `crates/srv/src/backend/rpc_types/editor_read.rs` and the generated TypeScript types | The new fields |
| `frontend/app/view/editor/` | The note bar (text and viewers), disabled save, *Check again* and *Try again*, re-check on window focus |

The Restart Manager needs the `windows` crate's `Win32_System_RestartManager` feature; check whether srv already depends on `windows` and which features it enables.

### 8.4 Tests

- `file_lock.rs` unit tests on Windows CI, which hold a file open from the test with each share mode (no sharing, read-only sharing, a `LockFileEx` range), and check: the classification, `write_blocked`, that `holders` names the test's own process, and that `read_unlocked` returns zeros exactly in the locked range.
- `readeditorfile`: each case gives the right `read_only_reason`; an unlocked file is unchanged.
- Frontend: the note's text for each reason, with and without a program name; save disabled; *Check again* clearing the note when the next read is clean.
- **A dev-build check:** a file opened exclusively from PowerShell (`[IO.File]::Open(path, 'Open', 'ReadWrite', 'None')`), one opened with `FileShare.Read`, and a workbook open in Excel.

## 9. Implementation

| File | Change |
|---|---|
| `frontend/app/view/editor/file-icons.ts` (new) | `fileIcon(name, isDir, expanded)` → `{ icon, tone }` |
| `frontend/app/view/editor/tree-keyboard.ts` (new) | Pure helpers: the visible-row list from the model, next/previous/parent/first-child, type-ahead matching, `samePath()` |
| `frontend/app/view/editor/file-tree.tsx` | Roles, roving `tabindex`, key handling, the selected and open rows, reveal, the icons |
| `frontend/app/view/editor/file-tree-model.ts` | `selectedPath`, `revealPath()` (expand ancestors) |
| `frontend/app/view/editor/editor-view.tsx` | Ctrl+Shift+E and Escape focus moves, the focused-region marker, previewing a file from ↑/↓ through the existing click path |
| `frontend/app/view/editor/editor-view.scss`, `frontend/app/theme.scss` | Tokens and styles from §3, §4 and §6 |
| `frontend/app/view/editor/viewers/image/image-zoom.ts`, `image-viewer.tsx` | `clickSteps()`, `zoomStep()`, click versus drag, the modifier, cursors; the double-click toggle removed |
| `docs/keybindings.md` | Tree keys, Ctrl+Shift+E, image clicks |
| `docs/specs/SPEC_EDITOR_FILE_TREE_2026-05-26.md` | Correct the status line's claim that keyboard navigation is in |

### Tests

- `file-icons.test.ts`: whole-name matches beat extensions, case doesn't matter, `.env.local` matches, unknown and extensionless files get the plain file icon, folders open and closed.
- `tree-keyboard.test.ts`: ↑/↓ across expanded folders, ← to the parent, → into a folder, Home and End, type-ahead wrapping, and `samePath` with mixed separators and drive-letter case.
- `file-tree.test.tsx`: roles and `aria-*`; one `tabindex="0"`; ↓ previews the next file after the debounce, not each row it passes; Enter moves focus to the editor; the open row is marked when its path differs only by separator.
- `image-zoom.test.ts`: the steps run from fit to `maxScale` in five; the next step up and down from a scale between steps; clamped at both ends.
- **A dev-build check before the PR:** screenshots of the tree on a dark and a light theme; the open file revealed after an agent opens it; ↓ previewing; Ctrl+Shift+E moving focus with the marker following; five clicks reaching the deepest zoom; Ctrl+click and ⌥/Alt+click zooming out.

### Order

1. The tree look, the open file and the keyboard, as one PR on `main`.
2. Click zoom, after #5016 merges, since its image viewer is in that PR.
3. Files another program is using (§8), after #5016 merges, since its probe is in that PR.

## 10. Decisions (confirmed by the owner, 2026-10-16)

1. **↑/↓ opens a preview** (§5.3), with the 100 ms debounce; Enter or Space opens the tab for good.
2. **Ctrl+Shift+E** (⇧⌘E) switches between the tree and the editor body (§5.4).
3. **Double-click zoom goes away** (§7.1). Fit stays on `0` and the Fit button, actual size on `1`.
4. Click zoom's five steps run from fit to `maxScale`, as specified; capping it for smaller steps (§7.2) waits until the owner has tried it.

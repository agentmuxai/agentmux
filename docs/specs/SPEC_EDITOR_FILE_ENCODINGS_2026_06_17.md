# Editor file encodings (beyond UTF-8)

**Date:** 2026-06-17
**Status:** Draft
**Author:** naki
**Component:** `agentmux-srv/src/server/websocket.rs` (`readeditorfile`/`writeeditorfile`), `frontend/app/view/editor/`

---

## 1. The problem (and the .ini question)

**Today the editor only handles UTF-8 and fails on anything else.** The read path is:

```rust
// websocket.rs:1559 (readeditorfile)
let content = std::fs::read_to_string(path)?;   // <- requires valid UTF-8, errors otherwise
```

`read_to_string` returns `InvalidData` ("stream did not contain valid UTF-8") for any non-UTF-8 byte sequence, so the file simply won't open. The write path is the mirror problem:

```rust
// writeeditorfile — writes the UTF-8 String as-is
std::fs::write(path, cmd.content)   // <- always UTF-8, silently changes a non-UTF-8 file's encoding
```

So even if we made reads tolerant, saving would re-encode the file to UTF-8 and corrupt round-trips.

### "If I open an .ini, what format is that?"

**`.ini` has no standardized encoding — it's just text.** In practice:
- On **Windows**, classic INI files are **ANSI / the system code page** — for Western locales that's **Windows-1252 (CP1252)**. The Win32 INI APIs (`GetPrivateProfileString`) read them as ANSI unless a UTF-16 BOM is present.
- Modern tools may write **UTF-8** (sometimes with a BOM) or **UTF-16 LE** (with BOM).

So an `.ini` you open could be CP1252, UTF-8, or UTF-16 — and the moment it contains a non-ASCII byte (a `–`, `é`, `™`, smart quote `0x92`, etc.) in CP1252, our `read_to_string` rejects it and the editor errors. Encoding can't be inferred from the extension; it must be **detected from the bytes**.

We want to open (and correctly save) a **wide variety of encodings**, not just UTF-8.

---

## 2. How others do it (reference)

- **VS Code:** Node `jschardet` (detection) + `iconv-lite` (decode/encode). Sniffs BOM first, optional heuristic guess (`files.autoGuessEncoding`), shows the encoding in the status bar, and offers **"Reopen with Encoding"** / **"Save with Encoding"**. Default via `files.encoding`.
- **The Encoding Standard (WHATWG):** the canonical list of web-platform encodings (UTF-8/16, windows-1252, ISO-8859-*, Shift_JIS, GBK, Big5, EUC-KR/JP, KOI8, …). Implemented in Rust by **`encoding_rs`** (Firefox's encoder/decoder) — the de-facto standard for this in Rust, also used by ripgrep.
- **Detection in Rust:** **`chardetng`** — Gecko's compact charset detector (the one Firefox ships), pairs with `encoding_rs`.

We mirror VS Code's model with the Rust equivalents — **no external binary, pure crates.**

---

## 3. Design

### 3.1 Crates (agentmux-srv)

- **`encoding_rs`** — decode bytes → `String` and encode `String` → bytes for any WHATWG encoding. Lossless for round-trips of supported encodings; has a documented replacement behavior for un-encodable chars.
- **`chardetng`** — byte-sniffing detector → an `encoding_rs::Encoding` guess (with a TLD/locale hint we can leave default).
- (optional) `encoding_rs_io` if we later stream large files.

### 3.2 Detection order (read)

1. **BOM sniff** (authoritative when present): `EF BB BF` → UTF-8; `FF FE` → UTF-16 LE; `FE FF` → UTF-16 BE; `FF FE 00 00`/`00 00 FE FF` → UTF-32. Strip the BOM from the decoded text; **remember it** so save can re-emit it.
2. **Valid UTF-8?** If the bytes are valid UTF-8, use UTF-8 (no BOM). This keeps the common case exact and fast.
3. **Heuristic** via `chardetng` over the byte sample → best-guess encoding (e.g. windows-1252, Shift_JIS).
4. **Fallback** to the configured default (`editor:encoding.default`, default **windows-1252** on Windows / UTF-8 elsewhere) when detection is low-confidence.

Decode with `encoding_rs` (lossy=replacement on malformed input, with a `had_errors` flag surfaced to the UI as a soft warning).

### 3.3 Backend API changes

**`readeditorfile`** → returns encoding metadata alongside the (always-UTF-8-on-the-wire) text:
```jsonc
{
  "content": "…UTF-8 text for the editor…",
  "encoding": "windows-1252",     // detected/used label (encoding_rs name)
  "bom": "none" | "utf-8" | "utf-16le" | "utf-16be",
  "line_ending": "lf" | "crlf",   // detected (see §3.5)
  "had_decode_errors": false,
  "read_only": false
}
```
Reads **bytes** (not `read_to_string`), runs §3.2, decodes to UTF-8 for transport.

**`writeeditorfile`** → accepts the encoding to write back in:
```jsonc
{ "path", "content" /*UTF-8*/, "encoding": "windows-1252", "bom": "none", "line_ending": "crlf" }
```
Re-encodes `content` from UTF-8 → target encoding via `encoding_rs`, re-emits the BOM if the file had one (or the user chose one), normalizes line endings to the file's convention, then writes bytes. If a character can't be represented in the target encoding, surface a clear error/warning (offer "save as UTF-8") rather than silently dropping it.

### 3.4 Per-tab encoding state (frontend)

The editor model stores the file's `encoding` + `bom` + `line_ending` per tab (from the read response). Save passes them straight back so the file **round-trips in its original encoding** by default. Changing them (reopen/save-with-encoding) updates this state.

### 3.5 Line endings (adjacent, include now)

Encoding work naturally exposes CRLF vs LF. Detect on read, preserve on write (don't silently convert a CRLF Windows .ini to LF). Same per-tab metadata + status-bar indicator. (Editor edits in `\n`; convert at the write boundary.)

### 3.6 CodeMirror interplay

None required. CodeMirror always edits **decoded Unicode** (JS string). Encoding is purely a byte↔text boundary concern at read/write in the backend. The editor buffer, find, markdown preview, etc. are unaffected.

---

## 4. UI (mirror VS Code)

- **Encoding indicator** in the editor pane (e.g. a small chip in the editor footer/status, near the LSP chip): shows `UTF-8`, `Windows-1252`, `UTF-16 LE`, plus `CRLF`/`LF`. Reflects the active tab.
- **Reopen with Encoding** — re-decode the on-disk bytes with a chosen encoding (fixes a mis-detected file without losing data, since we re-read bytes).
- **Save with Encoding** — convert and save in a chosen encoding (updates per-tab state).
- A soft banner when `had_decode_errors` ("This file was decoded with replacements — it may not be <encoding>. Reopen with another encoding?").
- **Settings:** `editor:encoding.default` (default windows-1252 on Windows, utf-8 elsewhere) and `editor:encoding.autodetect` (on by default).

---

## 5. Edge cases

- **BOM round-trip:** preserve presence/absence; never add a BOM to a file that didn't have one unless the user asks.
- **Binary files:** the existing "looks binary" guard stays in front of all this — encodings are for *text*.
- **Un-encodable chars on save:** if the edited text has chars outside the target charset (e.g. typed an emoji into a CP1252 file), warn and offer UTF-8 instead of lossy-writing.
- **Low-confidence detection:** fall back to default + flag it; the user can Reopen-with-Encoding.
- **Large files:** the 10 MB guard stays; detection samples a bounded prefix.

---

## 6. Phasing

1. **Stop failing (open anything).** Read bytes → BOM/UTF-8/`chardetng` detect → `encoding_rs` decode. `readeditorfile` returns `encoding`/`bom`/`line_ending`. Files that error today (ANSI `.ini`, UTF-16, Latin-1, Shift_JIS…) now open. *(~1 day)*
2. **Round-trip save.** `writeeditorfile` re-encodes to the file's original encoding + BOM + line endings; per-tab encoding state on the frontend. No more silent UTF-8 conversion. *(~1 day)*
3. **UI.** Encoding/line-ending indicator chip + Reopen-with-Encoding + Save-with-Encoding + decode-error banner + settings. *(~1–2 days)*

Phase 1 alone fixes "my `.ini` won't open." Phases 2–3 make encodings first-class and safe to edit.

---

## 7. Key references

| What | Location |
|---|---|
| Read path (strict UTF-8 today) | `agentmux-srv/src/server/websocket.rs:1559` (`readeditorfile`) |
| Write path (always UTF-8 today) | `agentmux-srv/src/server/websocket.rs` (`writeeditorfile`, ~1571) |
| Editor read/write callers | `frontend/app/view/editor/editor-model.ts` (`ReadEditorFileCommand`, `saveFile`/`WriteEditorFileCommand`) |
| Crates | `encoding_rs`, `chardetng` (crates.io; pure Rust) |
| Existing size/binary guards | `readeditorfile` 10 MB cap; editor-model binary-content refusal |

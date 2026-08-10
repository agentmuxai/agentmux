# Spec: Media pane v4 — agent-facing `OpenMedia` MCP tool

**Status:** implemented — PR #2398 (OpenMedia MCP tool); verified in code 2026-08-10.
**Author:** AgentY

> **2026-08-07 audit note:** Implemented — `OpenMedia` MCP tool is live in
> `agentmux-mcp/src/main.rs`. Status field was never updated. See
> `docs/reports/REPORT_DOCS_AND_DEAD_CODE_CLEANUP_AUDIT_2026_08_07.md`.
**Date:** 2026-08-03
**Related:** `docs/specs/SPEC_MEDIA_PANE_2026_07_26.md` (v1 — implemented,
PR #2299; this spec's target pane, unchanged here), `docs/specs/SPEC_MEDIA_PANE_V2_AGENT_WORKFLOW_GAPS_2026_07_28.md`,
`docs/specs/SPEC_MEDIA_PANE_V3_BROWSER_AND_CUSTOM_TRANSPORT_2026_07_29.md`
(neither v2 nor v3 touches agent-facing opening — both assume the pane is
already open and focus on what it shows once it is), `agentmux-mcp/src/main.rs`
(`OpenEditor` tool, the direct precedent this spec mirrors), `agentmux-srv/src/server/app_api/pane.rs`
(`build_pane_meta`, the actual gap — see "Research" below).

## Motivation

Real gap hit today, not speculative: reviewing a generated video
(`steamboat-rescue`'s `demo/full-story-through-shot37.webm`) with a human, the
natural move was to open it in AgentMux's own Media pane — built for exactly
this ("agent produces/edits media, human reviews it," per v1's own
motivation) — the same way `OpenEditor` already opens a code/text file next
to the conversation. No such tool exists. `OpenEditor` unconditionally
requests `view: "editor"`; there is no agent-facing way to request
`view: "media"` instead. The only paths to populate a Media pane today are a
human using the native "Open File" dialog inside the pane, or a previously
persisted `media:path` block meta value — nothing an agent can drive.

This is a narrow, mechanical gap: the Media pane itself (rendering,
live-update watcher, blob-URL auth workaround) is fully built and unaffected
by this spec. This is purely "give agents the same programmatic entry point
into the Media pane that `OpenEditor` already gives them into the Editor
pane."

## Research: the gap is server-side too, not just a missing MCP tool

An earlier pass at this research (a subagent, not verified against the
actual match arms) concluded the backend `pane/open` handler takes `view` as
an unvalidated passthrough string, so `view: "media"` would "likely already
work" and the only missing piece was the MCP tool wrapper. **That's wrong —
checked directly against the code, not re-derived from a comment
elsewhere:**

`agentmux-srv/src/server/app_api/pane.rs:211-258`'s `build_pane_meta` matches
`cmd.view.as_str()` against a **closed whitelist**:

```rust
match cmd.view.as_str() {
    "editor" => { ... }
    "term" => { ... }
    "browser" => { ... }
    "sysinfo" => { ... }
    "help" => { ... }
    other => {
        return Err(format!(
            "INVALID_VIEW: unsupported view '{other}' (expected editor/term/browser/sysinfo/help)"
        ));
    }
}
```

`"media"` isn't in that list — a `pane/open` POST with `view: "media"` fails
today with a 400 `INVALID_VIEW` error (`agentmux-srv/src/server/mod.rs:804`
maps the `INVALID_VIEW`-prefixed error to that status), regardless of
whether an MCP tool sends it. **The gap is two pieces, not one**: this
whitelist needs a `"media"` arm, and the MCP tool needs to exist to send the
request. Confirmed both are needed by reading `build_pane_meta` directly
before writing this design, since the frontend's `MediaViewModel` (fully
built, per v1) made it easy to wrongly assume the whole path was already
wired.

**Confirmed the frontend side needs zero changes.** `frontend/app/view/media/media.tsx:24,191-197`:
the pane's `onMount` already reads a persisted `META_PATH = "media:path"`
block-meta key and calls `showPath(saved)` if present — exactly the
mechanism `build_pane_meta`'s new arm needs to populate. `showPath` (`media.tsx:169-174`)
always expects a **file** path, deriving its containing directory via
`dirnameOf()` and starting a directory watch automatically (v1's live-update
mechanism) — so `OpenMedia`'s `file` argument needs no separate
directory-vs-file mode; it's a single file path, matching `OpenEditor`'s
contract exactly.

## Non-goals

- No changes to `media.tsx` itself, the gallery/EDL/transport-bar work from
  v2/v3, the watcher, or the thumbnail/transcode ideas — this spec is
  strictly the agent-facing entry point, not the pane's rendering or review
  features.
- No directory-mode argument (e.g. "open this pane following the latest file
  in this folder") — per Research above, the existing meta contract is
  always a specific file path, and its directory-watch/follow behavior is
  automatic once a file is shown. Adding an explicit directory-target
  argument would be new pane-level scope belonging to v2 §2's Pin/Follow
  mode design, not this tool-wiring spec.
- No `collapse_tree`-equivalent argument. `OpenEditor`'s `collapse_tree`
  exists because the Editor pane has a file-tree sidebar
  (`editor:tree_expanded` meta); the Media pane has no tree UI, so there's
  no equivalent state to expose.
- No gallery/EDL/transport-bar/watcher changes to `media.tsx` — the one
  frontend change this spec does make (§3 below) is limited to the pane's
  header-title fallback, added only because this spec's own tool
  description otherwise promises a default that doesn't exist (see
  "Corrections from automated review" below); it is not a step toward v2/v3.

## Design

### 1. Backend: whitelist a `"media"` view in `build_pane_meta`

`agentmux-srv/src/server/app_api/pane.rs:211`, new arm alongside `"editor"`
(mirroring its required-`file` validation, but writing to `media:path`
instead of `file` — the two panes use different meta key names, confirmed
above):

```rust
"media" => {
    let file = cmd.file.as_deref().filter(|s| !s.is_empty())
        .ok_or_else(|| "MISSING_ARG: view=media requires 'file'".to_string())?;
    meta.insert("view".to_string(), json!("media"));
    meta.insert("media:path".to_string(), json!(file));
}
```

Update the `INVALID_VIEW` error's expected-list message
(`pane.rs:255`) to include `media`, so a caller typo'ing the view string
still gets an accurate hint.

No change needed to `CommandPaneOpenData`/`PaneOpenRequest` — both already
carry a generic `file: Option<String>` field (`agentmux-common/src/api_types.rs:143`),
reused as-is.

### 2. New MCP tool: `OpenMedia`

`agentmux-mcp/src/main.rs`, mirroring `OPEN_EDITOR_TOOL`
(`main.rs:208-222`) and its handler (`main.rs:794-847`ish), minus
`collapse_tree` per the Non-goals above:

```rust
const OPEN_MEDIA_TOOL: &str = r#"{
  "name": "OpenMedia",
  "description": "Open an image, video, or audio file in an AgentMux media pane next to this conversation. Use when you want the user to see/watch generated media you're discussing. Pass an absolute host path. Fire-and-forget: returns once the pane is opened.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "file":     { "type": "string", "description": "Absolute path to the media file to open" },
      "title":    { "type": "string", "description": "Optional tab/pane title (defaults to the file name)" },
      "split":    { "type": "string", "enum": ["right", "left", "down", "up"], "description": "Where to place the new pane relative to this agent pane (default: right). Ignored when floating is true." },
      "floating": { "type": "boolean", "description": "Open the file in a floating window (a chromeless pane over the app) instead of a docked split. Default: false." }
    },
    "required": ["file"]
  }
}"#;
```

Handler (`"OpenMedia" => { ... }` arm, same shape as `"OpenEditor"`'s):
identical `file` extraction/validation, identical `local_url`/`auth_key`
precondition check, identical split/floating resolution, building
`PaneOpenRequest { view: "media".to_string(), file: Some(file.to_string()),
focus: Some(true), split_direction, split_reference_block_id, title,
tree_expanded: None, floating, url: None, ... }` and POSTing to
`{local_url}/api/v1/pane/open` — same endpoint `OpenEditor` already uses,
now valid for `view: "media"` once §1 lands.

Parse it alongside the other tool consts (`let open_media: Value =
serde_json::from_str(OPEN_MEDIA_TOOL).expect("static json");`, `main.rs:510`
area) and add `open_media` to **both** of the two independent places a tool
must be listed — confirmed by reading both, not assumed to be one shared
list (see "Corrections from automated review" below):

1. **The actual production registration** — the hand-written array literal
   inside the `json!({...})` response built by the `"tools/list" => { ... }`
   match arm, `main.rs:513`
   (`"result": { "tools": [shell, shell_stop, ..., open_editor, ..., identity_validate] }`).
   This is what MCP clients actually receive; omitting `open_media` here
   means the tool is implemented but undiscoverable.
2. **The test-side `defs` array**, `main.rs:1719-1747`
   (`all_tool_defs_are_valid_json_with_names`) — a separate, independently
   maintained list that validates every tool const is well-formed JSON with
   a `name`/`inputSchema`, and pins the total count via
   `assert_eq!(defs.len(), 26, ...)`. This array does **not** drive
   production behavior and would keep passing at the old count even if
   step 1 above were skipped — it catches a malformed def, not a missing
   registration. Add `OPEN_MEDIA_TOOL` here too (bumping the asserted count
   to 27) for the JSON-validity coverage, but do not treat updating this
   array as a substitute for updating the real list in step 1.

### 3. Frontend: minimal `viewName` fallback (so the tool's own promised default works)

`MediaViewModel` needs one small addition: a path-derived `viewName`,
mirroring `EditorViewModel`'s existing pattern exactly
(`frontend/app/view/editor/editor-model.ts:290-296`, `useBlockAtom` wrapping
a `createMemo` over the tracked file-path signal, falling back to a static
label — `"Editor"` there — when no path is set yet):

```ts
this.viewName = useBlockAtom(blockId, "media-view-name", () =>
    createMemo<string>(() => {
        const p = displayPath();
        return p ? basenameOf(p) : "Media";
    }),
);
```

(`basenameOf` — a small new helper alongside the existing `dirnameOf`, or
the tail-end of `dirnameOf`'s own split logic reused.) This is the one
frontend change this spec makes, scoped narrowly to the header-title
fallback `OpenMedia`'s own description promises — not a step toward v2/v3's
larger `media.tsx` work (see the Non-goals entry added above).

## Corrections from automated review (Codex, pre-merge)

Both real, confirmed by reading the exact lines named, not assumed correct
from the finding alone:

1. **Registration site was wrong.** The original draft of Design §2 pointed
   only at `main.rs:1719-1725` — that array is `#[cfg(test)]`-gated unit-test
   data, not the production `tools/list` handler. The actual response is a
   separate hand-written array literal at `main.rs:513`. Fixed above: both
   locations are now named, with the production one (§2 point 1) called out
   as the one that actually matters for discoverability, and the test array
   (§2 point 2) kept only for its own JSON-validity/count-pinning purpose.
2. **Tool description promised an unbuilt default.** `OPEN_MEDIA_TOOL`'s
   `title` property description ("defaults to the file name") was copied
   from `OPEN_EDITOR_TOOL` without checking that the underlying default
   exists for Media too. It doesn't: `EditorViewModel.viewName` derives the
   tab title from the file path (`editor-model.ts:290-296`); `MediaViewModel`
   has no `viewName` at all (confirmed via grep — zero matches), so every
   default-titled `OpenMedia` pane would show the generic "Media" label,
   indistinguishable from any other. Fixed by adding the minimal `viewName`
   fallback in Design §3 above, rather than just softening the description —
   the default is a small, real usability need (multiple review panes open
   at once), not scope worth cutting.

## Open questions

1. **Tool description wording for image vs. video vs. audio** — `OpenEditor`'s
   description says "file you're discussing or editing"; the draft above
   says "generated media you're discussing." Worth a second pass once this
   is actually used a few times to see if callers need a stronger hint
   about which extensions the pane supports (per `media.tsx`'s
   `IMAGE_EXTENSIONS`/`VIDEO_EXTENSIONS`/`AUDIO_EXTENSIONS` lists), or if a
   generic description reads fine in practice. Leans toward keeping it
   generic for v1 and only adding an extension hint if real usage shows
   confusion.
2. **Should `OpenMedia` reject unsupported extensions before opening the
   pane**, or let it open and show `media.tsx`'s own existing
   unsupported-extension error state? Leans toward the latter — matches
   `OpenEditor`'s own posture of not pre-validating file existence/type
   before opening, and avoids duplicating `media.tsx`'s extension list in
   two places that could drift.

## Files (anticipated — this spec does not implement)

| File | Relevance |
|------|-----------|
| `agentmux-srv/src/server/app_api/pane.rs:211-258` | `build_pane_meta` — add the `"media"` match arm (§1); the actual server-side gap, not just a missing MCP tool |
| `agentmux-mcp/src/main.rs:208-222` | `OPEN_EDITOR_TOOL` — pattern `OPEN_MEDIA_TOOL` copies |
| `agentmux-mcp/src/main.rs:794-847`(ish) | `"OpenEditor"` handler arm — pattern the new `"OpenMedia"` arm copies |
| `agentmux-mcp/src/main.rs:513` | **The actual `tools/list` production registration** — add `open_media` to this array. Missing this is the P1 Codex finding; the tool would exist but be undiscoverable without it |
| `agentmux-mcp/src/main.rs:1719-1747` | Test-only `defs` array + count assertion — add `OPEN_MEDIA_TOOL`, bump `assert_eq!` to 27. Validity/count coverage only, not a substitute for `main.rs:513` |
| `frontend/app/view/media/media.tsx:24,169-197` | `media:path` meta read on mount — confirmed unchanged, the target this spec's backend arm populates |
| `frontend/app/view/media/media.tsx` (new) | Add `viewName` (§3) — fixes the P2 Codex finding (tool description promised a file-name default title that didn't exist) |
| `frontend/app/view/editor/editor-model.ts:290-296` | `EditorViewModel.viewName` — the exact pattern §3's `MediaViewModel.viewName` mirrors |
| `docs/specs/SPEC_MEDIA_PANE_2026_07_26.md` | v1 — this spec adds an agent entry point to the pane it defines, no other change |

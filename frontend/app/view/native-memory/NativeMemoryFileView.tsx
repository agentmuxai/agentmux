// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * NativeMemoryFileView — Memory → Personal's full view of one
 * memory file (the third drill-down screen). Replaces the read-only
 * NativeMemoryHistoryPanel there, per
 * docs/specs/SPEC_MEMORY_FOLLOWS_THE_AGENT_2026_09_24.md §2.3/§2.4:
 *
 *   - Editing is added, through the existing `agent:memory:write_file` RPC
 *     (reversing SPEC_ARMORY_PERSONAL_MEMORY_CONTENT_VIEW_2026_09_11.md §6,
 *     approved by the repo owner);
 *   - pinned layout: action bar, banner and history in the top region, the
 *     content/editor filling the bottom;
 *   - a save carries `base_sha256`;
 *   - a live change no longer remounts this view (it used to be keyed on a
 *     `refreshNonce` bumped by every `agent:memory:changed`, which would wipe
 *     an open draft). `refreshNonce` is a prop now: each bump reloads history
 *     and content IN PLACE, and the draft model decides whether that's a
 *     silent update, a rebase, or the "changed since you started editing"
 *     banner (only when the file's hash differs from the draft's base).
 *
 * Remounts per (agentId, filename) — the caller keys it, as before.
 * The state is in native-memory-file-view-state.ts, the markup in native-memory-file-view-parts.tsx.
 */

import type { JSX } from "solid-js";
import { PinnedEditorLayout } from "@/app/view/memory-editor/PinnedEditorLayout";
import { FileBottom, FileTop } from "./native-memory-file-view-parts";
import { createNativeMemoryFile, type NativeMemoryFileViewProps } from "./native-memory-file-view-state";

export function NativeMemoryFileView(props: NativeMemoryFileViewProps): JSX.Element {
    const t = createNativeMemoryFile(props);
    const top = <FileTop t={t} confirm={props.confirm} />;
    const bottom = <FileBottom t={t} />;
    return <PinnedEditorLayout surface="armory-personal" top={top} bottom={bottom} onKeyDown={t.onKeyDown} />;
}

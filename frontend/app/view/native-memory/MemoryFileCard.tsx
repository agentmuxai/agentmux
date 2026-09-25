// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * MemoryFileCard — one *.md file in the Armory → Memory → Personal file grid.
 *
 * This grid replaced the file `<select>` that used to sit in the detail
 * header. The dropdown showed nothing but the filename, so choosing a file
 * meant opening it to find out what it was — the same "select each in turn to
 * see anything" problem the agent `<select>` had before
 * SPEC_ARMORY_PERSONAL_MEMORY_AGENT_BLOCKS_2026_09_01.md replaced it with
 * cards. A tile can carry the metadata the list RPC already returns
 * (`is_index`, `metadata_type`, `size_bytes`, `modified_at`), so the grid is
 * scannable without drilling in.
 *
 * Mirrors MemoryAgentCard's visual language and interaction contract
 * (role="button" + Enter/Space, not a native <button>) on purpose, since the
 * two grids are consecutive screens of the same drill-down. It is a separate
 * component for the same reason MemoryAgentCard is separate from AgentCard:
 * the two carry different payloads and neither should be able to break the
 * other by changing its own.
 *
 * See docs/specs/SPEC_ARMORY_PERSONAL_MEMORY_FILE_TILES_2026_09_04.md.
 *
 * Since 2026-09-24 a thin wrapper over the generic MemoryTile, which the
 * Global Memory tile grid shares
 * (SPEC_MEMORY_FOLLOWS_THE_AGENT_2026_09_24.md §2.3). The markup and classes
 * are unchanged.
 */

import type { JSX } from "solid-js";
import type { NativeMemoryFileMeta } from "@/app/store/rpc-api";
import { MemoryTile, type MemoryTileBadge } from "./MemoryTile";

interface MemoryFileCardProps {
    file: NativeMemoryFileMeta;
    /** Opens this file's version history. */
    onSelect: (filename: string) => void;
}

/** Byte-count label. Mirrors BundleImportPreviewModal's own formatBytes —
 *  duplicated rather than shared because that one is a private helper of an
 *  unrelated modal; a shared util is worth extracting only once a third
 *  caller appears. */
export function formatFileSize(bytes: number): string {
    if (!Number.isFinite(bytes) || bytes < 0) return "";
    if (bytes < 1024) return `${bytes} B`;
    if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
    return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

/** Relative modified-time label, same buckets as AgentLaunchModal's
 *  formatRelative. Returns "" for a missing/zero timestamp so the caller can
 *  omit the segment entirely rather than render a bogus "56y ago" for epoch
 *  zero. */
export function formatFileAge(ms: number): string {
    if (!ms || !Number.isFinite(ms)) return "";
    const delta = Date.now() - ms;
    if (delta < 0) return "just now";
    if (delta < 60_000) return "just now";
    if (delta < 3_600_000) return `${Math.floor(delta / 60_000)}m ago`;
    if (delta < 86_400_000) return `${Math.floor(delta / 3_600_000)}h ago`;
    return `${Math.floor(delta / 86_400_000)}d ago`;
}

/** The tile's meta line ("439 B · 2d ago"). Pulled out so the render and the
 *  aria-label can't drift, and so the empty-segment cases are unit-testable
 *  on their own. */
export function fileMetaLabel(file: NativeMemoryFileMeta): string {
    return [formatFileSize(file.size_bytes), formatFileAge(file.modified_at)].filter(Boolean).join(" · ");
}

export const MemoryFileCard = (props: MemoryFileCardProps): JSX.Element => {
    // The index file is the one loaded into every session, so it earns a
    // marker the other tiles don't get.
    const badges = (): MemoryTileBadge[] => [
        ...(props.file.is_index ? [{ label: "index", variant: "accent" as const }] : []),
        ...(props.file.metadata_type ? [{ label: props.file.metadata_type }] : []),
    ];
    return (
        <MemoryTile
            icon={props.file.is_index ? "fa-list" : "fa-file-lines"}
            title={props.file.filename}
            badges={badges()}
            meta={fileMetaLabel(props.file)}
            onSelect={() => props.onSelect(props.file.filename)}
            classList={{ "memory-file-card--index": props.file.is_index }}
            testId="memory-file-card"
            data={{ filename: props.file.filename }}
            ariaLabel={`${props.file.filename} — ${fileMetaLabel(props.file)}`}
        />
    );
};

MemoryFileCard.displayName = "MemoryFileCard";

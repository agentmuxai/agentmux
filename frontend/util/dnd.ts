// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/*
 * Shared drag-and-drop helpers for Terminal and Agent panes.
 *
 * The HTML5 drop event in CEF doesn't expose full filesystem paths — only
 * the bare File objects with display names. The Rust CefDragHandler captures
 * the real paths on OnDragEnter and stashes them; we read them back here via
 * `consume_drag_paths` and feed them to the per-file copy IPC.
 *
 * Spec: docs/specs/SPEC_PANE_FILE_DROP_2026_05_30.md §3.3, §3.4, §3.7.
 */

import { invokeCommand } from "@/app/platform/ipc";

export interface DropOutcome {
    /** Source paths that were attempted. */
    sources: string[];
    /** Per-source result: destination path on success, error string on failure. */
    results: Array<{ source: string; dest?: string; error?: string }>;
}

const DEFAULT_CONCURRENCY = 4;

/**
 * Read the OS paths captured by the CEF DragHandler stash. Returns an empty
 * array if the stash has expired or wasn't populated (non-CEF host, drop
 * with no files, etc.). The caller is expected to fall back gracefully.
 */
export async function consumeDragPaths(): Promise<string[]> {
    try {
        const paths = await invokeCommand<string[]>("consume_drag_paths", {});
        return Array.isArray(paths) ? paths : [];
    } catch {
        return [];
    }
}

/**
 * Copy each `sourcePath` into `targetDir` with the configured concurrency.
 * Filename collisions are de-conflicted server-side (`report (1).csv` etc.);
 * the returned `dest` is the actual landed path.
 */
export async function copyFilesToDir(
    sourcePaths: string[],
    targetDir: string,
    opts?: { concurrency?: number },
): Promise<DropOutcome> {
    const concurrency = Math.max(1, opts?.concurrency ?? DEFAULT_CONCURRENCY);
    const results: DropOutcome["results"] = [];

    let nextIdx = 0;
    const worker = async () => {
        while (true) {
            const i = nextIdx++;
            if (i >= sourcePaths.length) return;
            const source = sourcePaths[i];
            try {
                const dest = await invokeCommand<string>("copy_file_to_dir", {
                    sourcePath: source,
                    targetDir,
                });
                results[i] = { source, dest };
            } catch (err: unknown) {
                results[i] = { source, error: String(err) };
            }
        }
    };

    const workers = Array.from(
        { length: Math.min(concurrency, sourcePaths.length) },
        () => worker(),
    );
    await Promise.all(workers);
    return { sources: sourcePaths, results };
}

/** Extract a display filename from an absolute path (cross-platform). */
export function baseName(p: string): string {
    const m = p.match(/[^\\/]+$/);
    return m ? m[0] : p;
}

/** True if the drag event carries at least one native OS file (not just text/URL). */
export function isFileDrag(e: DragEvent): boolean {
    const types = e.dataTransfer?.types;
    return !!types && Array.from(types).includes("Files");
}

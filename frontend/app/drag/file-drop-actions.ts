// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/*
 * What a pane does with dropped files, written once: the working-folder
 * lookup, the copy into it (host paths, or the files' bytes when there are no
 * paths), the `@name` mentions, and the notices.
 * docs/specs/SPEC_DRAG_AND_DROP_CONSOLIDATION_2026_09_27.md §5.3.
 */

import { MOS, pushNotification } from "@/app/store/global";
import { baseName, copyFilesToDir } from "@/util/dnd";
import { uploadFileToWorkdir } from "../view/agent/attachments/attachment-draft";

/** The pane's working folder (`cmd:cwd`), if it has one. */
export function paneWorkdir(blockId: string): string | undefined {
    const block = MOS.getObjectValue<Block>(MOS.makeORef("block", blockId));
    const cwd = block?.meta?.["cmd:cwd"];
    return typeof cwd === "string" && cwd ? cwd : undefined;
}

export function fileCount(n: number): string {
    return `${n} ${n === 1 ? "file" : "files"}`;
}

function notice(type: "info" | "warning" | "error", title: string, message = "", seconds = 8): void {
    pushNotification({
        icon: type === "info" ? "fa-check" : "fa-triangle-exclamation",
        title,
        message,
        timestamp: new Date().toISOString(),
        type,
        expiration: Date.now() + seconds * 1000,
    });
}

/** The drop and copy notices, one builder each. */
export const notifyDrop = {
    noWorkdir: (paneKind: string) => notice("warning", "Drop failed", `No working directory detected for this ${paneKind}.`),
    noPaths: (count: number) =>
        notice("warning", "Drop failed", `Couldn't read the paths of ${fileCount(count)} dropped. Try again.`, 6),
    attachFailed: (err: unknown) => notice("warning", "Couldn't attach the files", String((err as Error)?.message ?? err)),
    /**
     * `attached`: `@name`s were inserted; `mention-yourself`: an agent pane
     * that didn't insert them; `to-dir`: a pane with no composer (terminal).
     */
    copied: (dests: string[], failures: string[], cwd: string, style: "attached" | "mention-yourself" | "to-dir") => {
        const verb = style === "attached" ? "Attached" : "Copied";
        const what = dests.length === 1 ? baseName(dests[0]) : `${dests.length} files`;
        const title = style === "to-dir" ? `${verb} ${what} to ${cwd}` : `${verb} ${what}`;
        const hint = style === "mention-yourself" ? `Files are in ${cwd}. Mention them in your next message.` : "";
        notice(
            failures.length > 0 ? "warning" : "info",
            failures.length > 0 ? `${title} (${failures.length} failed)` : title,
            failures.join("\n") || hint,
            failures.length > 0 ? 8 : 5
        );
    },
    /** A document tab dropped on another pane that couldn't take it. */
    cantMove: (title: string, reason: string) => notice("warning", `Couldn't move ${title}`, reason),
    /** A media or editor pane was handed a file it can't open. */
    cantOpen: (name: string, paneKind: string) =>
        notice("warning", "Couldn't open the file", `${name} can't be opened in this ${paneKind}.`),
    copyFailed: (failures: string[]) =>
        notice("error", `Copy failed (${fileCount(failures.length)})`, failures.join("\n"), 12),
};

export type CopySource = { paths: string[] } | { files: File[] };

/** `Promise.allSettled` over `items`, with at most `limit` running at a time. */
export async function settleWithLimit<T, R>(
    items: T[],
    limit: number,
    run: (item: T) => Promise<R>
): Promise<PromiseSettledResult<R>[]> {
    const results: PromiseSettledResult<R>[] = new Array(items.length);
    let next = 0;
    const worker = async () => {
        while (next < items.length) {
            const i = next++;
            try {
                results[i] = { status: "fulfilled", value: await run(items[i]) };
            } catch (reason) {
                results[i] = { status: "rejected", reason };
            }
        }
    };
    await Promise.all(Array.from({ length: Math.max(1, Math.min(limit, items.length)) }, worker));
    return results;
}

export interface CopyOptions {
    /**
     * Agent panes: insert `@name` for each copied file into this composer
     * root (null when the setting is off or the composer isn't mounted).
     * Leave both unset for panes without a composer.
     */
    mentionIn?: HTMLElement | null;
    splice?: (root: HTMLElement, tokens: string[]) => boolean;
    /** `dnd:concurrency`; absent means unlimited, as for copyFilesToDir. */
    concurrency?: number;
    /** What to call the pane in notices ("agent pane", "terminal pane"). */
    paneKind: string;
}

/**
 * Copy files into the pane's working folder and report the result.
 * Paths go through the host's copy; bytes (no host paths) are uploaded and
 * placed with `attachments.copy-to-workdir`. Returns the destinations.
 */
export async function copyIntoWorkdir(blockId: string, source: CopySource, opts: CopyOptions): Promise<string[]> {
    const cwd = paneWorkdir(blockId);
    if (!cwd) {
        notifyDrop.noWorkdir(opts.paneKind);
        return [];
    }
    const dests: string[] = [];
    const failures: string[] = [];
    if ("paths" in source) {
        const outcome = await copyFilesToDir(source.paths, cwd, { concurrency: opts.concurrency });
        for (const r of outcome.results) {
            if (r.dest) dests.push(r.dest);
            else if (r.error) failures.push(`${baseName(r.source)}: ${r.error}`);
        }
    } else {
        const results = await settleWithLimit(source.files, opts.concurrency ?? source.files.length, (f) =>
            uploadFileToWorkdir(blockId, f)
        );
        results.forEach((r, i) => {
            if (r.status === "fulfilled") dests.push(r.value);
            else failures.push(`${source.files[i].name || "file"}: ${String((r.reason as Error)?.message ?? r.reason)}`);
        });
    }
    let style: "attached" | "mention-yourself" | "to-dir" = "to-dir";
    if (opts.splice) {
        const mentioned =
            dests.length > 0 && !!opts.mentionIn && opts.splice(opts.mentionIn, dests.map((d) => `@${baseName(d)}`));
        style = mentioned ? "attached" : "mention-yourself";
    }
    if (dests.length > 0) notifyDrop.copied(dests, failures, cwd, style);
    else if (failures.length > 0) notifyDrop.copyFailed(failures);
    return dests;
}

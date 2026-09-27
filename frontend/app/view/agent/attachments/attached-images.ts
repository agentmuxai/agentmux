// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/*
 * The backend appends an `<attached_files>` block (`<attached_images>` before
 * any file could be attached) to a message with attachments
 * (agentmux-srv backend/attachments/prompt.rs `list_block`) and that text is
 * what gets stored. On replay this splits it back off, so the transcript
 * shows the user's own text with the thumbnails above it, not the list the
 * agent read. Spec §6.6–§6.7; SPEC_AGENT_PANE_FILE_ATTACHMENTS_2026_09_26.md §7.
 *
 *   look at these
 *
 *   <attached_files>
 *   The user attached 4 files. …
 *   1. a.png — C:\…\attachments\derived\ab\<sha256>.v1-e2000.send.png
 *   3. c.pdf [PDF, 2 pages; text version: /…/<sha256>.v1-e2000.text.txt] — /…/named/<sha256>/c.pdf
 *   4. d.png — /…/<sha256>.v1-e2000.send.jpg
 *   - b.png — (no longer available)
 *   </attached_files>
 */

import type { StripAttachment } from "./AttachmentStrip";

const BLOCK_RE = /\n*<(attached_files|attached_images)>\n([\s\S]*?)\n?<\/\1>\s*$/;
const FOUND_RE = /^(\d+)\. (.*) — (.+)$/;
const MISSING_RE = /^- (.*) — \(no longer available\)$/;
// A derived file (`<sha256>.….send.png`) or a named copy (`named/<sha256>/<name>`).
const ID_RE = /(?:^|[\\/])([0-9a-f]{64})\.[^\\/]*$|[\\/]named[\\/]([0-9a-f]{64})[\\/][^\\/]+$/;

/** `name [note]` → `name`; the note is the last bracketed part. */
function stripNote(nameAndNote: string): string {
    if (!nameAndNote.endsWith("]")) return nameAndNote;
    const at = nameAndNote.lastIndexOf(" [");
    return at > 0 ? nameAndNote.slice(0, at) : nameAndNote;
}

export function splitAttachedImages(message: string): { text: string; attachments: StripAttachment[] } {
    const m = BLOCK_RE.exec(message);
    if (!m) return { text: message, attachments: [] };
    const found = new Map<number, StripAttachment>();
    const missing: StripAttachment[] = [];
    for (const line of m[2].split("\n")) {
        const f = FOUND_RE.exec(line);
        if (f) {
            const idm = ID_RE.exec(f[3].trim());
            const id = idm?.[1] ?? idm?.[2];
            const name = stripNote(f[2]);
            found.set(Number(f[1]), id ? { id, name } : { name });
            continue;
        }
        const x = MISSING_RE.exec(line);
        if (x) missing.push({ name: x[1] });
    }
    if (found.size === 0 && missing.length === 0) return { text: message, attachments: [] };
    // Found images keep their numbers; missing ones fill the gaps in order.
    const total = Math.max(found.size + missing.length, ...found.keys());
    const attachments: StripAttachment[] = [];
    let next = 0;
    for (let n = 1; n <= total; n++) {
        const a = found.get(n) ?? missing[next++];
        if (a) attachments.push(a);
    }
    return { text: message.slice(0, m.index).trimEnd(), attachments };
}

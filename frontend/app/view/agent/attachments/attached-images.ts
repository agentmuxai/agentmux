// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/*
 * The backend appends an `<attached_images>` block to a message with images
 * (agentmux-srv backend/attachments/prompt.rs `list_block`) and that text is
 * what gets stored. On replay this splits it back off, so the transcript
 * shows the user's own text with the thumbnails above it, not the list the
 * agent read. Spec §6.6–§6.7.
 *
 *   look at these
 *
 *   <attached_images>
 *   The user attached 3 images. …
 *   1. a.png — C:\…\attachments\derived\ab\<sha256>.v1-e2000.send.png
 *   3. c.png — /…/<sha256>.v1-e2000.send.jpg
 *   - b.png — (no longer available)
 *   </attached_images>
 */

import type { StripAttachment } from "./AttachmentStrip";

const BLOCK_RE = /\n*<attached_images>\n([\s\S]*?)\n?<\/attached_images>\s*$/;
const FOUND_RE = /^(\d+)\. (.*) — (.+)$/;
const MISSING_RE = /^- (.*) — \(no longer available\)$/;
const ID_RE = /(?:^|[\\/])([0-9a-f]{64})\.[^\\/]*$/;

export function splitAttachedImages(message: string): { text: string; attachments: StripAttachment[] } {
    const m = BLOCK_RE.exec(message);
    if (!m) return { text: message, attachments: [] };
    const found = new Map<number, StripAttachment>();
    const missing: StripAttachment[] = [];
    for (const line of m[1].split("\n")) {
        const f = FOUND_RE.exec(line);
        if (f) {
            const id = ID_RE.exec(f[3].trim())?.[1];
            found.set(Number(f[1]), id ? { id, name: f[2] } : { name: f[2] });
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

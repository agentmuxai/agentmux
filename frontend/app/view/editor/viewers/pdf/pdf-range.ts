// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * One range of a large PDF, for pdf.js's range transport. pdf.js has no way
 * to hear that a range failed and would wait for it forever, so a failure
 * goes to `fail` and the viewer shows it instead of a blank page.
 */
export async function loadRange(
    fetchRange: (begin: number, endInclusive: number) => Promise<{ blob: Blob | null }>,
    begin: number,
    end: number,
    deliver: (begin: number, bytes: Uint8Array) => void,
    fail: (message: string) => void
): Promise<void> {
    try {
        const r = await fetchRange(begin, end - 1);
        // No blob: the server sent the whole file instead of the part asked for.
        if (!r.blob) throw new Error("the file can't be read in parts any more");
        deliver(begin, new Uint8Array(await r.blob.arrayBuffer()));
    } catch (e) {
        fail(`Couldn't read part of this PDF: ${e instanceof Error ? e.message : String(e)}`);
    }
}

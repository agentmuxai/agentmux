// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Draws the JPEG frames of a streamed pane on its canvas at their own size, decoding off the main thread
// (`createImageBitmap`). A frame that arrives while one is being decoded replaces any frame waiting, so a slow
// decode shows the newest picture, not a queue of old ones. docs/specs/SPEC_STREAMED_BROWSER_PANE_DEMO_2026_11_15.md §7.

export interface FramePainter {
    draw(jpeg: Uint8Array): void;
}

export function createFramePainter(canvas: () => HTMLCanvasElement | undefined): FramePainter {
    let busy = false;
    let waiting: Uint8Array | null = null;

    async function paint(jpeg: Uint8Array): Promise<void> {
        busy = true;
        try {
            const bitmap = await createImageBitmap(new Blob([jpeg as unknown as BlobPart], { type: "image/jpeg" }));
            const target = canvas();
            if (target) {
                // The canvas is the frame's own size: never stretched, so a click lands where the person sees it.
                if (target.width !== bitmap.width || target.height !== bitmap.height) {
                    target.width = bitmap.width;
                    target.height = bitmap.height;
                }
                target.getContext("2d")?.drawImage(bitmap, 0, 0);
            }
            bitmap.close();
        } catch {
            // A frame that cannot be decoded is skipped; the next one replaces it.
        }
        busy = false;
        const next = waiting;
        waiting = null;
        if (next) void paint(next);
    }

    return {
        draw(jpeg) {
            if (busy) waiting = jpeg;
            else void paint(jpeg);
        },
    };
}

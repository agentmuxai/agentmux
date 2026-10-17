// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { acquireViewerSignal } from "./pdf-viewer-signal";

describe("acquireViewerSignal", () => {
    it("aborts only when the last viewer closes", () => {
        const a = acquireViewerSignal();
        const b = acquireViewerSignal();
        expect(b.signal).toBe(a.signal);
        a.release();
        // The first viewer closing must not take the shared listeners with it.
        expect(b.signal.aborted).toBe(false);
        b.release();
        expect(a.signal.aborted).toBe(true);
    });

    it("starts a fresh signal for the next viewer, and a second release is harmless", () => {
        const a = acquireViewerSignal();
        a.release();
        a.release();
        const b = acquireViewerSignal();
        expect(b.signal.aborted).toBe(false);
        expect(b.signal).not.toBe(a.signal);
        b.release();
        expect(b.signal.aborted).toBe(true);
    });
});

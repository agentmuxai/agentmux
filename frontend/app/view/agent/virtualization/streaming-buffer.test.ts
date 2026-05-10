// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import type { DocumentNode } from "../types";
import {
    locateIndex,
    partitionForVirtualization,
    STREAMING_BUFFER_SIZE,
} from "./streaming-buffer";

const md = (id: string): DocumentNode => ({ type: "markdown", id, content: id, timestamp: 0 });

const range = (n: number): DocumentNode[] =>
    Array.from({ length: n }, (_, i) => md(`n${i}`));

describe("partitionForVirtualization", () => {
    it("puts everything in streamingNodes when document <= bufferSize", () => {
        const nodes = range(10);
        const p = partitionForVirtualization(nodes, 50);
        expect(p.virtualizedNodes).toHaveLength(0);
        expect(p.streamingNodes).toBe(nodes); // identity — no slicing
        expect(p.splitIndex).toBe(0);
    });

    it("splits at length - bufferSize when document > bufferSize", () => {
        const nodes = range(70); // 50 streaming, 20 virtualized
        const p = partitionForVirtualization(nodes, 50);
        expect(p.virtualizedNodes).toHaveLength(20);
        expect(p.streamingNodes).toHaveLength(50);
        expect(p.splitIndex).toBe(20);
        // Virtualized side is the head (oldest); streaming side is the tail (newest).
        expect(p.virtualizedNodes[0].id).toBe("n0");
        expect(p.virtualizedNodes[19].id).toBe("n19");
        expect(p.streamingNodes[0].id).toBe("n20");
        expect(p.streamingNodes[49].id).toBe("n69");
    });

    it("handles exactly bufferSize nodes (boundary)", () => {
        const nodes = range(50);
        const p = partitionForVirtualization(nodes, 50);
        expect(p.virtualizedNodes).toHaveLength(0);
        expect(p.streamingNodes).toBe(nodes);
        expect(p.splitIndex).toBe(0);
    });

    it("handles bufferSize+1 (smallest case where virtualization activates)", () => {
        const nodes = range(51);
        const p = partitionForVirtualization(nodes, 50);
        expect(p.virtualizedNodes).toHaveLength(1);
        expect(p.streamingNodes).toHaveLength(50);
        expect(p.virtualizedNodes[0].id).toBe("n0");
    });

    it("uses STREAMING_BUFFER_SIZE as default", () => {
        const nodes = range(STREAMING_BUFFER_SIZE + 10);
        const p = partitionForVirtualization(nodes);
        expect(p.virtualizedNodes).toHaveLength(10);
        expect(p.streamingNodes).toHaveLength(STREAMING_BUFFER_SIZE);
    });

    it("handles empty document", () => {
        const p = partitionForVirtualization([]);
        expect(p.virtualizedNodes).toHaveLength(0);
        expect(p.streamingNodes).toHaveLength(0);
        expect(p.splitIndex).toBe(0);
    });
});

describe("locateIndex", () => {
    it("locates indices in the virtualized region", () => {
        const p = partitionForVirtualization(range(70), 50);
        // splitIndex=20; index 0 is virtualized, relativeIndex=0.
        expect(locateIndex(0, p)).toEqual({ side: "virtualized", relativeIndex: 0 });
        expect(locateIndex(19, p)).toEqual({ side: "virtualized", relativeIndex: 19 });
    });

    it("locates indices in the streaming region", () => {
        const p = partitionForVirtualization(range(70), 50);
        // splitIndex=20; index 20 is streaming side, relativeIndex 0.
        expect(locateIndex(20, p)).toEqual({ side: "streaming", relativeIndex: 0 });
        expect(locateIndex(69, p)).toEqual({ side: "streaming", relativeIndex: 49 });
    });

    it("returns null for out-of-range indices", () => {
        const p = partitionForVirtualization(range(70), 50);
        expect(locateIndex(-1, p)).toBeNull();
        expect(locateIndex(70, p)).toBeNull();
        expect(locateIndex(999, p)).toBeNull();
    });

    it("handles empty partition", () => {
        const p = partitionForVirtualization([]);
        expect(locateIndex(0, p)).toBeNull();
    });

    it("locates the only index in a single-node streaming partition", () => {
        const p = partitionForVirtualization(range(1));
        expect(locateIndex(0, p)).toEqual({ side: "streaming", relativeIndex: 0 });
    });
});

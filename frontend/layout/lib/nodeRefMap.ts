// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

export class NodeRefMap {
    private map: Map<string, { current: HTMLDivElement | null }> = new Map();
    generation: number = 0;

    set(id: string, ref: { current: HTMLDivElement | null }) {
        this.map.set(id, ref);
        this.generation++;
    }

    delete(id: string) {
        if (this.map.has(id)) {
            this.map.delete(id);
            this.generation++;
        }
    }

    get(id: string): { current: HTMLDivElement | null } {
        if (this.map.has(id)) {
            return this.map.get(id);
        }
    }
}

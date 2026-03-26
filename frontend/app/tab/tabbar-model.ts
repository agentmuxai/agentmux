// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createSignal } from "solid-js";
import type { SignalAtom } from "@/util/util";

export class TabBarModel {
    private static instance: TabBarModel | null = null;

    jigglePinAtom: SignalAtom<number>;

    private constructor() {
        const [get, set] = createSignal(0);
        const atom = () => get();
        (atom as any)._set = set;
        this.jigglePinAtom = atom as unknown as SignalAtom<number>;
    }

    static getInstance(): TabBarModel {
        if (!TabBarModel.instance) {
            TabBarModel.instance = new TabBarModel();
        }
        return TabBarModel.instance;
    }

    jiggleActivePinnedTab() {
        this.jigglePinAtom._set((prev) => prev + 1);
    }
}

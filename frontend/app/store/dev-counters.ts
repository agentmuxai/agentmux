// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Counters (dev tooling) — split out of global.ts (see global.ts's
// "Counters (dev tooling)" section for the original context). Re-exported
// from global.ts for backward-compat (97 files import from that module).

const Counters = new Map<string, number>();

export function countersClear() {
    Counters.clear();
}

export function counterInc(name: string, incAmt = 1) {
    let count = Counters.get(name) ?? 0;
    count += incAmt;
    Counters.set(name, count);
}

export function countersPrint() {
    let outStr = "";
    for (const [name, count] of Counters.entries()) {
        outStr += `${name}: ${count}\n`;
    }
    console.log(outStr);
}

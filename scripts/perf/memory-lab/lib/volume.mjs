// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// How full the volume under the lab's data folder is. An SSD slows badly as it fills (its cache runs out), and
// the operator's longest store-lock holds (5 to 11.6 s, 14-17 October) all fell on days the drive had 4 to 6% free
// and none since 238 GB was freed (#5141): a lab run that stalls should say how full its disk was.

import { statfsSync } from "node:fs";

/** Total and free space, in GB and percent free, of the volume holding `path`; null when the OS will not say. */
export function volumeOf(path) {
    try {
        const s = statfsSync(path);
        const total = s.blocks * s.bsize;
        const free = s.bavail * s.bsize;
        if (!(total > 0)) return null;
        const gb = (b) => Math.round((b / 1073741824) * 10) / 10;
        return { totalGB: gb(total), freeGB: gb(free), freePct: Math.round((free / total) * 1000) / 10 };
    } catch {
        return null;
    }
}

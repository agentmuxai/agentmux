// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { afterEach, describe, expect, it } from "vitest";
import { getPlatform, setPlatform } from "@/util/platformutil";
import { revealInFileManagerLabel } from "./file-manager-label";

describe("revealInFileManagerLabel", () => {
    const original = getPlatform();
    afterEach(() => setPlatform(original));

    it.each([
        ["win32", "Reveal in File Explorer"],
        ["darwin", "Reveal in Finder"],
        ["linux", "Reveal in file manager"],
    ] as const)("names the file manager on %s", (platform, label) => {
        setPlatform(platform);
        expect(revealInFileManagerLabel()).toBe(label);
    });
});

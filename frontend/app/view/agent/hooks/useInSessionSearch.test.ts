// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createRoot } from "solid-js";
import { describe, expect, it } from "vitest";
import { recycleBlockers } from "@/app/store/recycle-blockers";
import { useInSessionSearch } from "./useInSessionSearch";

describe("useInSessionSearch", () => {
    it("an open search blocks a renderer recycle until it is closed", () => {
        const { search, dispose } = createRoot((d) => ({ search: useInSessionSearch({ document: () => [] }), dispose: d }));
        search.setVisible(true);
        expect(recycleBlockers()).toContain("a transcript search is open");
        search.close();
        expect(recycleBlockers()).not.toContain("a transcript search is open");
        dispose();
    });
});

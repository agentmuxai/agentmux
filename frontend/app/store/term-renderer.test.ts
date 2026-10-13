// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { claimTermRenderer, describeTermRenderer, forgetTermRenderer, setTermRendererOf, termRendererOf } from "./term-renderer";

describe("term renderer, per terminal", () => {
    it("keeps each terminal's renderer apart", () => {
        setTermRendererOf("a", { kind: "webgl" });
        setTermRendererOf("b", { kind: "dom", why: "WebGL is turned off (term:disablewebgl)" });
        expect(termRendererOf("a")?.kind).toBe("webgl");
        expect(termRendererOf("b")?.kind).toBe("dom");
        forgetTermRenderer("a");
        expect(termRendererOf("a")).toBeUndefined();
        expect(termRendererOf("b")?.kind).toBe("dom");
    });

    it("only the newest mount of a block sets or clears its renderer", () => {
        const oldMount = {};
        const newMount = {};
        claimTermRenderer("c", oldMount);
        setTermRendererOf("c", { kind: "webgl", owner: oldMount });
        // The block mounts again before the old mount is disposed.
        claimTermRenderer("c", newMount);
        setTermRendererOf("c", { kind: "webgl", owner: newMount });
        // The old mount's canvas still loses its context, then the old mount closes.
        setTermRendererOf("c", { kind: "lost", owner: oldMount });
        forgetTermRenderer("c", oldMount);
        expect(termRendererOf("c")?.kind).toBe("webgl");
        forgetTermRenderer("c", newMount);
        expect(termRendererOf("c")).toBeUndefined();
        // Nothing can bring it back once the claim is gone.
        setTermRendererOf("c", { kind: "dom", owner: newMount });
        expect(termRendererOf("c")).toBeUndefined();
    });

    it("describes each renderer for the hover panel", () => {
        expect(describeTermRenderer({ kind: "webgl" }).value).toBe("WebGL (GPU)");
        expect(describeTermRenderer({ kind: "dom" }).value).toBe("DOM (software)");
        expect(describeTermRenderer({ kind: "dom", why: "WebGL couldn't start for this terminal" }).meaning).toBe(
            "WebGL couldn't start for this terminal"
        );
        expect(describeTermRenderer({ kind: "lost" }).value).toBe("WebGL, lost");
    });
});

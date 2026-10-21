// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it, vi } from "vitest";

vi.mock("@/util/endpoints", () => ({ getWebServerEndpoint: () => "http://srv" }));

import { computeBgStyleFromMeta } from "./muxutil";

const bg = (value: string) => computeBgStyleFromMeta({ bg: value } as MetaType)?.background;

describe("computeBgStyleFromMeta", () => {
    it("uses a background that loads nothing as written, without the parser", () => {
        expect(bg("#123456")).toBe("#123456");
        expect(bg("background: linear-gradient(red, blue);")).toBe("linear-gradient(red, blue)");
    });

    it("waits for the parser for anything that can load, then vets and rewrites it", async () => {
        // Nothing until the parser has loaded (a memo computes it again then).
        expect(bg("url(/home/me/a.png)")).toBeNull();
        expect(bg('image-set("a.png" 1x)')).toBeNull();
        await vi.waitFor(() => expect(bg("url(/home/me/a.png)")).not.toBeNull());
        expect(bg("url(/home/me/a.png)")).toContain("http://srv/agentmux/stream-file?path=");
        expect(bg("url(https://example.com/a.png)")).toContain("https://example.com/a.png");
        // A relative or unknown-scheme URL is still refused.
        expect(bg("url(a.png)")).toBeNull();
        expect(bg("url(ftp://example.com/a.png)")).toBeNull();
        expect(bg('url("javascript:void 0")')).toBeNull();
    });
});

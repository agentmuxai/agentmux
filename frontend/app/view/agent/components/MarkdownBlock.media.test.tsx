// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The agent's own messages show local images inline, resolving relative paths
 * against the working directory the pane provides; outside an agent pane
 * nothing loads (SPEC_AGENT_PANE_RICH_OUTPUT_2026_09_27.md §4.1).
 */

import { cleanup, render, waitFor } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const h = vi.hoisted(() => ({ fetch: vi.fn() }));
vi.mock("@/util/fetchutil", () => ({ fetch: (...a: unknown[]) => h.fetch(...a) }));
vi.mock("@/util/endpoints", () => ({ getWebServerEndpoint: () => "http://srv" }));
vi.mock("@/app/store/app-api", () => ({ getApi: () => ({ getAuthKey: () => "k" }) }));

import { AgentMediaProvider, AgentPaneProviders } from "../agent-media";
import type { MarkdownNode } from "../types";
import { MarkdownBlock } from "./MarkdownBlock";

const node: MarkdownNode = { type: "markdown", id: "m1", content: "After: ![toolbar](shots/after.png)" };

beforeEach(() => {
    h.fetch.mockReset().mockImplementation(async () => ({
        ok: true,
        headers: new Headers({ "Content-Length": "4" }),
        blob: async () => new Blob(["png!"], { type: "image/png" }),
    }));
    vi.stubGlobal("URL", Object.assign(URL, { createObjectURL: () => "blob:mb", revokeObjectURL: () => {} }));
});
afterEach(() => {
    cleanup();
    vi.unstubAllGlobals();
});

describe("MarkdownBlock inline media", () => {
    it("loads a relative image against the pane's working directory", async () => {
        const { container } = render(() => (
            <AgentMediaProvider baseDir={() => "D:/proj"}>
                <MarkdownBlock node={node} />
            </AgentMediaProvider>
        ));
        await waitFor(() => expect(container.querySelector("img")?.getAttribute("src")).toBe("blob:mb"));
        expect(new URL(h.fetch.mock.calls[0][0] as string).searchParams.get("path")).toBe("D:/proj/shots/after.png");
    });

    it.each([
        [{ meta: { "cmd:cwd": "D:/launched" } }, { working_directory: "D:/defined" }, "D:/launched/shots/after.png"],
        [{ meta: {} }, { working_directory: "D:/defined" }, "D:/defined/shots/after.png"],
    ])("the pane's providers prefer the launch cwd over the definition's directory", async (block, agent, want) => {
        const { container } = render(() => (
            <AgentPaneProviders dormant={() => false} block={() => block} agent={() => agent}>
                <MarkdownBlock node={node} />
            </AgentPaneProviders>
        ));
        await waitFor(() => expect(container.querySelector("img")).not.toBeNull());
        expect(new URL(h.fetch.mock.calls[0][0] as string).searchParams.get("path")).toBe(want);
    });

    it("loads nothing outside an agent pane", () => {
        const { container } = render(() => <MarkdownBlock node={node} />);
        expect(container.querySelector("img")).toBeNull();
        expect(h.fetch).not.toHaveBeenCalled();
    });
});

// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { GITHUB_PRESETS, githubPresetById, githubPresetPatch, githubRepoOf, isGithubRepo } from "./github-presets";

describe("GitHub presets", () => {
    it("fills the method, the full api.github.com URL, GitHub's headers and the body", () => {
        const patch = githubPresetPatch(githubPresetById("rerun-failed")!, "agentmuxai/agentmux")!;
        expect(patch["method"]).toBe("POST");
        expect(patch["url"]).toBe("https://api.github.com/repos/agentmuxai/agentmux/actions/runs/{{trigger.run_id}}/rerun-failed-jobs");
        expect(patch["headers"]).toMatchObject({ Accept: "application/vnd.github+json", "User-Agent": "AgentMux-Drone" });
        expect(patch["body"]).toBe("");
        expect(patch).not.toHaveProperty("auth");
    });

    it("sends a JSON content type with a body, and keeps comment text inside its JSON string", () => {
        const comment = githubPresetPatch(githubPresetById("comment")!, "a/b")!;
        expect(comment["headers"]).toMatchObject({ "Content-Type": "application/json" });
        expect(comment["body"]).toContain("| jsonstr}}");
        const read = githubPresetPatch(githubPresetById("pull")!, "a/b")!;
        expect(read["headers"]).not.toHaveProperty("Content-Type");
    });

    it("lets only owner/name into a URL", () => {
        for (const bad of ["", "agentmux", "a/b/c", "a/b?x=1", "a b/c", "../x", "a/..", "./.", "a/b#c", "a@evil.com/b"]) {
            expect(isGithubRepo(bad), bad).toBe(false);
            expect(githubPresetPatch(GITHUB_PRESETS[0], bad), bad).toBeNull();
        }
        expect(isGithubRepo(" agentmuxai/agentmux ")).toBe(true);
        expect(isGithubRepo("a.b/c-d_e")).toBe(true);
    });

    it("marks the writes, and every preset names the one GitHub site with no reference in the host", () => {
        expect(GITHUB_PRESETS.filter((p) => p.writes).map((p) => p.id).sort()).toEqual(["comment", "merge", "rerun-failed"]);
        for (const p of GITHUB_PRESETS) {
            const url = githubPresetPatch(p, "o/r")!["url"] as string;
            expect(url.startsWith("https://api.github.com/repos/o/r/"), p.id).toBe(true);
            expect(p.method === "GET" || p.writes, `${p.id}: only a read uses GET`).toBe(true);
        }
    });

    it("reads the repository back out of a URL", () => {
        expect(githubRepoOf("https://api.github.com/repos/agentmuxai/agentmux/pulls/5")).toBe("agentmuxai/agentmux");
        expect(githubRepoOf("https://api.github.com/repos/o/r")).toBe("o/r");
        expect(githubRepoOf("https://example.com/repos/o/r")).toBe("");
        expect(githubRepoOf("")).toBe("");
    });
});

// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { getCliCatalogEntry } from "./cli-catalog";
import {
    containerImageNote,
    imageBlocksContainer,
    shouldPreselectContainer,
    type PreselectInput,
} from "./container-default";

const ok: PreselectInput = {
    templateType: "container",
    containerSupported: true,
    dockerAvailable: true,
    image: "public",
};

describe("shouldPreselectContainer", () => {
    it("picks container when everything works", () => {
        expect(shouldPreselectContainer(ok)).toBe(true);
    });

    it("accepts a locally cached image and an unknown answer", () => {
        expect(shouldPreselectContainer({ ...ok, image: "local" })).toBe(true);
        expect(shouldPreselectContainer({ ...ok, image: "unknown" })).toBe(true);
    });

    it("falls back to host when the registry refuses the image or does not have it", () => {
        expect(shouldPreselectContainer({ ...ok, image: "denied" })).toBe(false);
        expect(shouldPreselectContainer({ ...ok, image: "not_found" })).toBe(false);
    });

    it("waits for the image check instead of guessing", () => {
        expect(shouldPreselectContainer({ ...ok, image: undefined })).toBe(false);
    });

    it("never picks container without Docker, without CLI support, or for a host template", () => {
        expect(shouldPreselectContainer({ ...ok, dockerAvailable: false })).toBe(false);
        expect(shouldPreselectContainer({ ...ok, containerSupported: false })).toBe(false);
        expect(shouldPreselectContainer({ ...ok, templateType: "host" })).toBe(false);
        expect(shouldPreselectContainer({ ...ok, templateType: undefined })).toBe(false);
    });
});

describe("imageBlocksContainer", () => {
    it("blocks only on a definite refusal", () => {
        expect(imageBlocksContainer("denied")).toBe(true);
        expect(imageBlocksContainer("not_found")).toBe(true);
        for (const s of ["local", "public", "unknown", undefined] as const) {
            expect(imageBlocksContainer(s)).toBe(false);
        }
    });
});

describe("containerImageNote", () => {
    it("says why, and what happens, when the image is blocked", () => {
        expect(containerImageNote("denied")).toMatch(/refuses access/);
        expect(containerImageNote("denied")).toMatch(/host/);
        expect(containerImageNote("not_found")).toMatch(/wasn't found/);
    });

    it("says nothing otherwise", () => {
        for (const s of ["local", "public", "unknown", undefined] as const) {
            expect(containerImageNote(s)).toBeNull();
        }
    });
});

describe("the default container image", () => {
    const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "../../../../..");
    const base = "ghcr.io/agentmuxai/agent-base:latest";

    it("is the same public base image in the catalog, the seed and the server", () => {
        expect(getCliCatalogEntry("claude")?.containerImage).toBe(base);

        const seed = JSON.parse(readFileSync(resolve(repoRoot, "crates/srv/agent-seed.json"), "utf8"));
        const claude = seed.agents.find((a: { id: string }) => a.id === "claude");
        expect(claude.container_image).toBe(base);

        const rust = readFileSync(resolve(repoRoot, "crates/srv/src/backend/container_image.rs"), "utf8");
        expect(rust).toContain(`pub const DEFAULT_AGENT_IMAGE: &str = "${base}";`);
    });

    it("is not the private image that bundles Claude Code", () => {
        const catalog = readFileSync(resolve(repoRoot, "frontend/app/view/agent/defaults/cli-catalog.ts"), "utf8");
        const seed = readFileSync(resolve(repoRoot, "crates/srv/agent-seed.json"), "utf8");
        expect(catalog).not.toContain("agent-claude");
        expect(seed).not.toContain("agent-claude");
    });
});

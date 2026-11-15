// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import path from "node:path";
import { describe, expect, it } from "vitest";
import { makeWatchIgnore, UNWATCHED_TOP_DIRS } from "./vite-watch-ignore.mjs";

const root = path.resolve("/work/clone");
const at = (...parts) => path.join(root, ...parts);

describe("makeWatchIgnore", () => {
    const ignore = makeWatchIgnore(root);

    it("skips the directories the dev server never serves", () => {
        for (const dir of UNWATCHED_TOP_DIRS) {
            expect(ignore(at(dir))).toBe(true); // the directory itself too, so no watch is opened on it
            expect(ignore(at(dir, "a", "b.txt"))).toBe(true);
        }
    });

    it("skips an alternative cargo target directory", () => {
        expect(ignore(at("target-agent5", "debug", "deps", "x.o"))).toBe(true);
        expect(ignore(at("target-loap", "d1"))).toBe(true);
    });

    it("keeps the frontend sources and the other served directories", () => {
        expect(ignore(at("frontend", "app", "App.tsx"))).toBe(false);
        expect(ignore(at("frontend", "target", "x.ts"))).toBe(false); // only a top-level target is build output
        expect(ignore(at("public", "icon.svg"))).toBe(false);
        expect(ignore(at("scripts", "dev-home.mjs"))).toBe(false);
        expect(ignore(at("index.html"))).toBe(false);
        expect(ignore(at("targets.txt"))).toBe(false);
    });

    it("leaves paths outside the root alone", () => {
        expect(ignore(path.resolve("/elsewhere/target/x"))).toBe(false);
        expect(ignore(`${root}-sibling/target/x`)).toBe(false);
    });

    it("accepts the forward-slashed form the watcher may hand over", () => {
        expect(ignore(at(".dev-home", "5300", "db").replace(/\\/g, "/"))).toBe(true);
    });

    it.runIf(process.platform === "win32")("ignores the case of the path on Windows", () => {
        expect(ignore(at(".dev-home", "5300").toUpperCase())).toBe(true);
        expect(makeWatchIgnore(root.toLowerCase())(at(".DEV-HOME", "5300"))).toBe(true);
    });
});

// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The headless web UI serves index.html under a policy that refuses inline script, inline event
// handlers, a base element and any address off its own origin (`content_security_policy` in
// crates/srv/src/server/local_session_routes.rs). A page that breaks one logs a violation on every
// load, and with violations expected, a new one would pass unseen: so none is let in.

import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const root = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
const html = readFileSync(join(root, "index.html"), "utf8");

describe("index.html under the headless web UI's policy", () => {
    it("has no inline script: every script is a file served from this origin", () => {
        const scripts = [...html.matchAll(/<script\b([^>]*)>([\s\S]*?)<\/script>/gi)];
        expect(scripts.length).toBeGreaterThan(0);
        for (const [tag, attrs, body] of scripts) {
            expect(attrs, tag).toMatch(/\bsrc="[^"]+"/);
            expect(body.trim(), `inline code in ${tag}`).toBe("");
            expect(attrs, tag).not.toMatch(/src="(?:[a-z]+:)?\/\//i);
        }
    });

    it("has no inline event handler, javascript: address or base element", () => {
        expect(html).not.toMatch(/\son[a-z]+\s*=/i);
        expect(html).not.toMatch(/javascript:/i);
        expect(html).not.toMatch(/<base\b/i);
    });

    it("loads no style, script, image or font from another origin", () => {
        const refs = [...html.matchAll(/\b(?:src|href)="([^"]+)"/gi)].map((m) => m[1]);
        expect(refs.length).toBeGreaterThan(0);
        for (const ref of refs) expect(ref, ref).not.toMatch(/^(?:[a-z][a-z0-9+.-]*:)?\/\//i);
    });

    it("keeps the early boot code in a file of public/, where the page's script tag points", () => {
        const src = /<script src="(\/early-boot\.js)"><\/script>/.exec(html)?.[1];
        expect(src).toBe("/early-boot.js");
        const code = readFileSync(join(root, "public", "early-boot.js"), "utf8");
        expect(code).toContain("window_transparent");
        expect(code).toContain("startup-loading-headline");
    });
});

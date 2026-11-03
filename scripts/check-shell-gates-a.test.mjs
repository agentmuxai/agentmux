// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// check-name-resolver-callers.sh, check-no-cef-version-in-overview.sh and check-long-jump.sh against a made-up repo.

import { chmodSync } from "node:fs";
import { join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { HAS_BASH, makeFixture } from "./lib/check-fixture.mjs";

const d = HAS_BASH ? describe : describe.skip;

let fx;
afterEach(() => fx?.remove());

function repo(gate, files) {
    fx = makeFixture([gate], files);
    return fx;
}

d("check-name-resolver-callers.sh", () => {
    const GATE = "check-name-resolver-callers.sh";

    it("passes when resolve_name_to_uid appears only in the two boundary files", () => {
        const r = repo(GATE, {
            "crates/srv/src/backend/name_resolution.rs": "pub fn resolve_name_to_uid() {}\n",
            "crates/srv/src/server/name_resolution.rs": "use x::resolve_name_to_uid;\n",
            "crates/srv/src/other.rs": "fn f() {}\n",
        }).run(GATE);
        expect(r.status).toBe(0);
        expect(r.out).toContain("ok");
    });

    it("fails on a caller anywhere else in the checked crates", () => {
        const r = repo(GATE, { "crates/mcp/src/x.rs": "fn f() { resolve_name_to_uid(n); }\n" }).run(GATE);
        expect(r.status).toBe(1);
        expect(r.out).toContain("crates/mcp/src/x.rs");
    });

    it("looks at .rs files only", () => {
        const r = repo(GATE, { "crates/srv/NOTES.md": "resolve_name_to_uid\n" }).run(GATE);
        expect(r.status).toBe(0);
    });
});

d("check-no-cef-version-in-overview.sh", () => {
    const GATE = "check-no-cef-version-in-overview.sh";

    it("passes on a README that says Chromium via CEF", () => {
        const r = repo(GATE, { "README.md": "AgentMux embeds Chromium via CEF.\n" }).run(GATE);
        expect(r.status).toBe(0);
    });

    it.each(["Built on CEF 154.", "Uses Chromium v154 under the hood.", "CEF milestone 154", "chromium-154"])("fails on %s", (line) => {
        const r = repo(GATE, { "README.md": `${line}\n` }).run(GATE);
        expect(r.status).toBe(1);
        expect(r.out).toContain("a CEF/Chromium version is named in the overview");
    });

    it("fails on a version in a diagram", () => {
        const r = repo(GATE, { "README.md": "ok\n", "assets/arch.svg": "<text>CEF 154</text>\n" }).run(GATE);
        expect(r.status).toBe(1);
        expect(r.out).toContain("assets/arch.svg");
    });

    it("fails on a version wrapped across two lines", () => {
        const r = repo(GATE, { "README.md": "It runs on CEF\n154 today.\n" }).run(GATE);
        expect(r.status).toBe(1);
        expect(r.out).toContain("wrapped across lines");
    });

    it("lets a number that is not a version through", () => {
        const r = repo(GATE, { "README.md": "CEF handles 20 panes; Chromium 3 tabs.\n" }).run(GATE);
        expect(r.status).toBe(0);
    });
});

d("check-long-jump.sh", () => {
    const GATE = "check-long-jump.sh";

    it("prints its usage and exits 2 without a full name/shared/channel triple", () => {
        const r = repo(GATE, {}).run(GATE, ["srv", "name"]);
        expect(r.status).toBe(2);
    });

    function fakeSrv(body) {
        fx.write("fake-srv.sh", `#!/bin/bash\n${body}\n`);
        chmodSync(join(fx.root, "fake-srv.sh"), 0o755);
        return join(fx.root, "fake-srv.sh");
    }

    it("passes when migrate and migrate --verify both succeed", () => {
        repo(GATE, {});
        const srv = fakeSrv('if [ "$4" = "--verify" ]; then echo "verify: 4 applied checked"; else echo \'{"applied":4,"skipped":0}\'; fi');
        const r = fx.run(GATE, [srv, "old", "-", fx.root]);
        expect(r.status).toBe(0);
        expect(r.out).toContain("migrate: ok, \"applied\":4,\"skipped\":0");
        expect(r.out).toContain("verify: 4 applied checked");
    });

    it("fails when migrate fails", () => {
        repo(GATE, {});
        const srv = fakeSrv("echo boom; exit 1");
        const r = fx.run(GATE, [srv, "old", "-", fx.root]);
        expect(r.status).toBe(1);
        expect(r.out).toContain("migrate: FAILED");
    });

    it("fails and prints the mismatch when verify finds one", () => {
        repo(GATE, {});
        const srv = fakeSrv('if [ "$4" = "--verify" ]; then echo "[MISMATCH] users"; exit 1; fi; echo ok');
        const r = fx.run(GATE, [srv, "old", "-", fx.root]);
        expect(r.status).toBe(1);
        expect(r.out).toContain("[MISMATCH] users");
    });
});

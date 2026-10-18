// @vitest-environment node
//
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// click_label.js, the rule act.rs's COMMIT_INFO names a committing click by,
// run as the same text the host splices in. The element's reads (aria-label,
// aria-labelledby's text, innerText) are passed as strings: jsdom has no
// innerText, and the rule itself is only about the strings.

import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const src = readFileSync(join(dirname(fileURLToPath(import.meta.url)), "click_label.js"), "utf8");
const clickLabel = new Function(`return (${src});`)();

describe("clickLabel", () => {
    it("names a menu item by its title and keeps its description apart", () => {
        const got = clickLabel(null, "", "Delete revision from history\nThis edit's content will no longer be visible");
        expect(got).toEqual({
            name: "Delete revision from history",
            description: "This edit's content will no longer be visible",
            verb: true,
        });
    });

    it("judges the name only: a payment word in the description doesn't make it a payment", () => {
        const got = clickLabel(null, "", "Archive invoice\nYou can still pay it later");
        expect(got.name).toBe("Archive invoice");
        expect(got.description).toBe("You can still pay it later");
        expect(got.verb).toBe(false);
    });

    it("a Delete whose description says it is safe still commits", () => {
        const got = clickLabel(null, "", "Delete\nThis is safe: it can be restored for 30 days");
        expect(got.name).toBe("Delete");
        expect(got.verb).toBe(true);
    });

    it("takes aria-label, then aria-labelledby, before the visible text", () => {
        expect(clickLabel("  Send   message ", "Ignored", "Go\nmore").name).toBe("Send message");
        expect(clickLabel("  Send   message ", "Ignored", "Go\nmore").description).toBe("");
        expect(clickLabel("", "Remove  member", "x").name).toBe("Remove member");
        expect(clickLabel(null, "", "  \n  Publish \n").name).toBe("Publish");
        expect(clickLabel(null, "", "").name).toBe("");
    });

    it("caps the name and the description", () => {
        const got = clickLabel(null, "", `${"a".repeat(300)}\n${"b".repeat(300)}`);
        expect(got.name).toHaveLength(120);
        expect(got.description).toHaveLength(200);
    });
});

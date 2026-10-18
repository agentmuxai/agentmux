// @vitest-environment node
//
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// click_label.js, the rule act.rs's COMMIT_INFO names a click by and decides
// whether it commits, run as the same text the host splices in. The
// element's reads (aria-label, aria-labelledby's text, innerText, and what it
// is) are passed as plain values: jsdom has no innerText, and the rule itself
// is only about those values.

import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const src = readFileSync(join(dirname(fileURLToPath(import.meta.url)), "click_label.js"), "utf8");
const clickLabel = new Function(`return (${src});`)();

const button = {};
const link = { link: true };

describe("clickLabel: the name and the description", () => {
    it("names a menu item by its title and keeps its description apart", () => {
        const got = clickLabel(null, "", "Delete revision from history\nThis edit's content will no longer be visible", button);
        expect(got.name).toBe("Delete revision from history");
        expect(got.description).toBe("This edit's content will no longer be visible");
    });

    it("takes aria-label, then aria-labelledby, before the visible text", () => {
        expect(clickLabel("  Send   message ", "Ignored", "Go\nmore", button).name).toBe("Send message");
        expect(clickLabel("  Send   message ", "Ignored", "Go\nmore", button).description).toBe("");
        expect(clickLabel("", "Remove  member", "x", button).name).toBe("Remove member");
        expect(clickLabel(null, "", "  \n  Publish \n", button).name).toBe("Publish");
        expect(clickLabel(null, "", "", button).name).toBe("");
    });

    it("caps the name and the description", () => {
        const got = clickLabel(null, "", `${"a".repeat(300)}\n${"b".repeat(300)}`, button);
        expect(got.name).toHaveLength(120);
        expect(got.description).toHaveLength(200);
    });
});

describe("clickLabel: whether a click commits, and why", () => {
    it("anything that submits a form commits", () => {
        expect(clickLabel(null, "", "Next", { submits: true })).toMatchObject({ committing: true, why: "it submits a form" });
    });

    it("a button commits on a committing word in its name or any later line", () => {
        expect(clickLabel(null, "", "Archive\nThis deletes it permanently", button)).toMatchObject({
            name: "Archive",
            committing: true,
            why: 'its text says "deletes"',
        });
        // A price over "Pay now": shown as "$20.00", still committing.
        expect(clickLabel(null, "", "$20.00\nPay now", button)).toMatchObject({
            name: "$20.00",
            description: "Pay now",
            committing: true,
        });
        // A plain Delete whose description says it is safe still commits.
        expect(clickLabel(null, "", "Delete\nThis is safe: it can be restored for 30 days", button)).toMatchObject({
            committing: true,
            why: 'its name says "Delete"',
        });
        expect(clickLabel(null, "", "Archive invoice\nMoves it out of your inbox", button).committing).toBe(false);
    });

    it("a link commits only when its name starts with a committing word or is short and holds one", () => {
        const ack =
            "This acknowledgement of receipt does not confirm the accuracy or completeness of the return. Click here to download a PDF";
        expect(clickLabel(null, "", ack, link)).toMatchObject({ committing: false, why: "" });
        expect(clickLabel(null, "", "Confirm order", link)).toMatchObject({ committing: true, why: 'its name says "Confirm"' });
        expect(clickLabel(null, "", "Yes, delete my account now", link).committing).toBe(true);
        expect(clickLabel(null, "", "Delete the old drafts in this folder and all of its subfolders", link).committing).toBe(true);
        expect(clickLabel(null, "", "Read more about how we handle the posts you delete", link).committing).toBe(false);
        // A link's description doesn't make it commit.
        expect(clickLabel(null, "", "Billing history\nPay an invoice", link).committing).toBe(false);
    });

    it("a download link or a file: link never commits", () => {
        expect(clickLabel(null, "", "Delete log.txt", { link: true, download: true }).committing).toBe(false);
        expect(clickLabel(null, "", "Submit form.pdf", { link: true, fileHref: true }).committing).toBe(false);
    });
});

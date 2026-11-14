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
        const got = clickLabel(null, "", `${"a".repeat(300)}\n${"b".repeat(1300)}`, button);
        expect(got.name).toHaveLength(120);
        expect(got.description).toHaveLength(1000);
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

describe("clickLabel: the accessible name", () => {
    // A button whose name comes from a title, an image's alt, a child's aria-label or CSS content has no text of its own.
    it("commits on the accessible name when the element's own text and aria-label say nothing", () => {
        expect(clickLabel(null, "", "", button).committing).toBe(false);
        const got = clickLabel(null, "", "", button, "Submit for certification");
        expect(got).toMatchObject({ committing: true, name: "Submit for certification", why: 'its name says "Submit"' });
    });

    it("keeps the element's own name and reason when it commits by itself", () => {
        const got = clickLabel(null, "", "Delete draft", button, "Trash");
        expect(got).toMatchObject({ committing: true, name: "Delete draft", why: 'its name says "Delete"' });
    });

    it("commits when either name says so", () => {
        expect(clickLabel(null, "", "Continue", button, "Pay now")).toMatchObject({ committing: true, name: "Continue", why: 'its name says "Pay"' });
        expect(clickLabel(null, "", "Continue", button, "Next step").committing).toBe(false);
    });

    it("applies the link rule to a link's accessible name", () => {
        const ack = "This acknowledgement of receipt does not confirm the accuracy or completeness of the return. Click here to download a PDF";
        expect(clickLabel(null, "", "", link, ack).committing).toBe(false);
        expect(clickLabel(null, "", "", link, "Confirm order").committing).toBe(true);
    });

    it("a link with text of its own keeps its description out of the name", () => {
        // The accessible name joins the title and the description: it must not make the description's word commit.
        expect(clickLabel(null, "", "Billing history\nPay an invoice", link, "Billing history Pay an invoice").committing).toBe(false);
        // A link with no text of its own is judged by its accessible name.
        expect(clickLabel(null, "", "", link, "Pay an invoice").committing).toBe(true);
    });

    it("an unnamed click whose accessible name could not be read asks", () => {
        expect(clickLabel(null, "", "", button, null)).toMatchObject({ committing: true, unreadable: true, why: "its name could not be read" });
        // One that has a name of its own is judged by it.
        expect(clickLabel(null, "", "Next", button, null)).toMatchObject({ committing: false });
        expect(clickLabel(null, "", "Next", button, null).unreadable).toBeUndefined();
    });

    it("an element the browser reads as unnamed is judged as before", () => {
        expect(clickLabel(null, "", "", button, "")).toMatchObject({ committing: false, name: "" });
        expect(clickLabel(null, "", "", { submits: true }, "")).toMatchObject({ committing: true, why: "it submits a form" });
    });
});

describe("clickLabel: a brand or plan with Pay in its name is not a payment (#47625)", () => {
    const asks = (name, el = button, ax) => clickLabel(null, "", name, el, ax).committing;

    it("a payment-method row, link, tab or toggle does not commit", () => {
        for (const name of ["Kakao Pay", "Cash App Pay", "Apple Pay", "Google Pay", "Amazon Pay", "APPLE PAY", "Samsung Pay"]) {
            expect(asks(name), name).toBe(false);
            expect(asks(name, link), `${name} (link)`).toBe(false);
        }
        expect(asks("Payment methods", link)).toBe(false);
        expect(asks("Payment methods")).toBe(false);
        expect(asks("Pay-as-you-go")).toBe(false);
        expect(asks("Pay-per-use plan")).toBe(false);
    });

    it("a row whose accessible name or description carries the brand does not commit", () => {
        expect(asks("Kakao Pay", button, "Kakao Pay")).toBe(false);
        expect(clickLabel(null, "", "Kakao Pay\nShow Kakao Pay to customers in Korea", button).committing).toBe(false);
    });

    it("a command still commits", () => {
        for (const name of [
            "Pay", "Pay now", "Pay $5.46", "Place order", "Confirm and Pay", "Click to Pay", "Pay with Apple Pay", "Buy now", "Buy with Apple Pay",
            "Confirm And Pay", "Proceed To Pay", "Continue To Pay", "Review And Pay", "Place Order And Pay", "CONFIRM AND PAY", "CLICK TO PAY",
            "Complete Pay", "Confirm Pay", "Pay Pay", "Confirm Apple Pay",
        ]) {
            expect(asks(name), name).toBe(true);
        }
        expect(clickLabel(null, "", "Pay", button).why).toBe('its name says "Pay"');
    });

    it("a brand next to a command still commits", () => {
        expect(asks("Kakao Pay", { submits: true })).toBe(true);
        expect(asks("Apple Pay\nPay $20.00 now")).toBe(true);
    });
});

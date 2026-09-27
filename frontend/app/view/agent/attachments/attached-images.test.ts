// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { splitAttachedImages } from "./attached-images";

const A = "a".repeat(64);
const C = "c".repeat(64);

describe("splitAttachedImages", () => {
    it("leaves ordinary messages alone", () => {
        expect(splitAttachedImages("hello")).toEqual({ text: "hello", attachments: [] });
    });

    it("splits the list off and reads ids from the send-copy paths", () => {
        const msg = [
            "look at these",
            "",
            "<attached_images>",
            "The user attached 3 images. The numbers match how the user refers to them.",
            `1. a.png — C:\\Users\\me\\.agentmux\\attachments\\derived\\aa\\${A}.v1-e2000.send.png`,
            `3. shot — 2 — final.png — /home/me/.agentmux/attachments/derived/cc/${C}.v1-e2000.send.jpg`,
            "- b.png — (no longer available)",
            "</attached_images>",
        ].join("\n");
        const r = splitAttachedImages(msg);
        expect(r.text).toBe("look at these");
        expect(r.attachments).toEqual([
            { id: A, name: "a.png" },
            { name: "b.png" },
            { id: C, name: "shot — 2 — final.png" },
        ]);
    });

    it("handles a message that is only images", () => {
        const msg = `<attached_images>\nThe user attached 1 image.\n1. x.png — /p/${A}.v1-e2000.send.png\n</attached_images>`;
        expect(splitAttachedImages(msg)).toEqual({ text: "", attachments: [{ id: A, name: "x.png" }] });
    });

    it("ignores a block the user merely typed mid-message", () => {
        const msg = "what is <attached_images>\nfoo\n</attached_images> doing here?";
        expect(splitAttachedImages(msg).attachments).toEqual([]);
    });
});

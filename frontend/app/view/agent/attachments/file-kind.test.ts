// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { attachmentNoun, extLabel, fileKind, hasThumbnail, kindIcon } from "./file-kind";

describe("fileKind", () => {
    it("trusts srv's kind over the name", () => {
        expect(fileKind("text", "renamed.pdf")).toBe("text");
        expect(fileKind("other", "notes.txt")).toBe("other");
    });

    it("guesses from the extension when srv's kind is unknown", () => {
        expect(fileKind(undefined, "Report.PDF")).toBe("pdf");
        expect(fileKind(undefined, "deck.pptx")).toBe("powerpoint");
        expect(fileKind(undefined, "sheet.ods")).toBe("excel");
        expect(fileKind(undefined, "main.rs")).toBe("text");
        expect(fileKind(undefined, "photo.heic")).toBe("image_file");
        expect(fileKind("", "Makefile")).toBe("other");
        expect(fileKind("future_kind", "a.png")).toBe("image");
    });
});

describe("tiles", () => {
    it("thumbnails only what previews cheaply and safely", () => {
        expect(["image", "svg", "text"].every((k) => hasThumbnail(fileKind(k, "")))).toBe(true);
        expect(["pdf", "word", "archive", "video", "image_file", "other"].some((k) => hasThumbnail(fileKind(k, "")))).toBe(
            false,
        );
    });

    it("picks brand icons, and csv/code/lines for text", () => {
        expect(kindIcon("pdf", "a.pdf")).toBe("fa-file-pdf");
        expect(kindIcon("word", "a.rtf")).toBe("fa-file-word");
        expect(kindIcon("text", "data.tsv")).toBe("fa-file-csv");
        expect(kindIcon("text", "app.tsx")).toBe("fa-file-code");
        expect(kindIcon("text", "README.md")).toBe("fa-file-lines");
        expect(kindIcon("other", "blob")).toBe("fa-file");
    });

    it("labels the extension", () => {
        expect(extLabel("report.docx")).toBe("DOCX");
        expect(extLabel("archive.tar.gz")).toBe("GZ");
        expect(extLabel("Makefile")).toBe("");
    });
});

describe("attachmentNoun", () => {
    it("says images only when every attachment is one", () => {
        expect(attachmentNoun(1, ["image"])).toBe("image");
        expect(attachmentNoun(3, ["image", "image", "image"])).toBe("images");
        expect(attachmentNoun(2, ["image", "pdf"])).toBe("attachments");
        expect(attachmentNoun(1, ["pdf"])).toBe("attachment");
        expect(attachmentNoun(0, [])).toBe("attachments");
    });
});

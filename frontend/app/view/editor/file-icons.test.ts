// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { fileIcon } from "./file-icons";

describe("fileIcon", () => {
    it("matches a whole file name before its extension", () => {
        expect(fileIcon("package.json", false)).toEqual({ icon: "fa-brands fa-npm", tone: "red" });
        expect(fileIcon("other.json", false)).toEqual({ icon: "fa-solid fa-file-code", tone: "yellow" });
        expect(fileIcon("Cargo.toml", false).icon).toBe("fa-brands fa-rust");
        expect(fileIcon("Dockerfile", false).icon).toBe("fa-brands fa-docker");
    });

    it("ignores case", () => {
        expect(fileIcon("README.MD", false)).toEqual(fileIcon("readme.md", false));
        expect(fileIcon("Main.PY", false)).toEqual({ icon: "fa-brands fa-python", tone: "blue" });
    });

    it("gives every .env file the key", () => {
        expect(fileIcon(".env", false)).toEqual({ icon: "fa-solid fa-key", tone: "yellow" });
        expect(fileIcon(".env.local", false)).toEqual(fileIcon(".env", false));
    });

    it("uses the language's own mark where Font Awesome has one", () => {
        expect(fileIcon("app.tsx", false).icon).toBe("fa-brands fa-react");
        expect(fileIcon("lib.rs", false)).toEqual({ icon: "fa-brands fa-rust", tone: "orange" });
        expect(fileIcon("index.html", false).icon).toBe("fa-brands fa-html5");
        expect(fileIcon("run.sh", false)).toEqual({ icon: "fa-solid fa-terminal", tone: "green" });
    });

    it("gives unknown and extensionless files the plain file icon", () => {
        const plain = { icon: "fa-solid fa-file", tone: "grey" };
        expect(fileIcon("notes.xyz", false)).toEqual(plain);
        expect(fileIcon("CHANGES", false)).toEqual(plain);
        expect(fileIcon(".bashrc", false)).toEqual(plain);
        expect(fileIcon("trailing.", false)).toEqual(plain);
    });

    it("draws folders closed and open, in steel", () => {
        expect(fileIcon("src", true)).toEqual({ icon: "fa-solid fa-folder", tone: "steel" });
        expect(fileIcon("src", true, true)).toEqual({ icon: "fa-solid fa-folder-open", tone: "steel" });
        // A folder named like a file is still a folder.
        expect(fileIcon("package.json", true).icon).toBe("fa-solid fa-folder");
    });
});

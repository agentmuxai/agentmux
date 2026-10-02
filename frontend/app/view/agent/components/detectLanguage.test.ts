// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { detectableLanguages, detectLanguage } from "./detectLanguage";

describe("detectLanguage", () => {
    // Extension map
    it("maps .ts to typescript", () => expect(detectLanguage("foo.ts")).toBe("typescript"));
    it("maps .tsx to tsx", () => expect(detectLanguage("foo.tsx")).toBe("tsx"));
    it("maps .js to javascript", () => expect(detectLanguage("foo.js")).toBe("javascript"));
    it("maps .jsx to jsx", () => expect(detectLanguage("foo.jsx")).toBe("jsx"));
    it("maps .py to python", () => expect(detectLanguage("foo.py")).toBe("python"));
    it("maps .rs to rust", () => expect(detectLanguage("foo.rs")).toBe("rust"));
    it("maps .go to go", () => expect(detectLanguage("foo.go")).toBe("go"));
    it("maps .sh to bash", () => expect(detectLanguage("foo.sh")).toBe("bash"));
    it("maps .ps1 to powershell", () => expect(detectLanguage("foo.ps1")).toBe("powershell"));
    it("maps .json to json", () => expect(detectLanguage("foo.json")).toBe("json"));
    it("maps .yaml to yaml", () => expect(detectLanguage("foo.yaml")).toBe("yaml"));
    it("maps .yml to yaml", () => expect(detectLanguage("foo.yml")).toBe("yaml"));
    it("maps .toml to toml", () => expect(detectLanguage("foo.toml")).toBe("toml"));
    it("maps .scss to scss", () => expect(detectLanguage("foo.scss")).toBe("scss"));
    it("maps .css to css", () => expect(detectLanguage("foo.css")).toBe("css"));
    it("maps .sql to sql", () => expect(detectLanguage("foo.sql")).toBe("sql"));
    it("maps .md to markdown", () => expect(detectLanguage("foo.md")).toBe("markdown"));
    it("maps .tf to terraform", () => expect(detectLanguage("foo.tf")).toBe("terraform"));
    it("maps .graphql to graphql", () => expect(detectLanguage("foo.graphql")).toBe("graphql"));

    // Absolute paths — extension still wins
    it("handles absolute paths", () =>
        expect(detectLanguage("/home/user/project/src/main.rs")).toBe("rust"));
    it("handles Windows paths", () =>
        expect(detectLanguage("C:\\Users\\dev\\app\\index.ts")).toBe("typescript"));

    // Basename matches
    it("maps Dockerfile to dockerfile", () => expect(detectLanguage("Dockerfile")).toBe("dockerfile"));
    it("maps dockerfile (lowercase) to dockerfile", () =>
        expect(detectLanguage("/app/dockerfile")).toBe("dockerfile"));
    it("maps Makefile to makefile", () => expect(detectLanguage("Makefile")).toBe("makefile"));
    it("leaves .gitignore plain: Shiki has no grammar for it", () => expect(detectLanguage(".gitignore")).toBe("text"));
    it("maps .env to dotenv", () => expect(detectLanguage(".env")).toBe("dotenv"));
    it("maps .env.local to dotenv", () => expect(detectLanguage(".env.local")).toBe("dotenv"));

    // Shebang detection
    it("detects python3 shebang", () =>
        expect(detectLanguage("script", "#!/usr/bin/env python3")).toBe("python"));
    it("detects bash shebang", () =>
        expect(detectLanguage("myscript", "#!/bin/bash")).toBe("bash"));
    it("detects /bin/sh shebang", () =>
        expect(detectLanguage("myscript", "#!/bin/sh")).toBe("bash"));
    it("detects node shebang", () =>
        expect(detectLanguage("script", "#!/usr/bin/env node")).toBe("javascript"));
    it("detects ruby shebang", () =>
        expect(detectLanguage("script", "#!/usr/bin/ruby")).toBe("ruby"));

    // Fallback
    it("returns text for unknown extension", () =>
        expect(detectLanguage("foo.unknown")).toBe("text"));
    it("returns text for no extension", () =>
        expect(detectLanguage("somefile")).toBe("text"));
    it("returns text for empty string", () =>
        expect(detectLanguage("")).toBe("text"));
});

describe("detectLanguage: every answer is a grammar the highlighter has", () => {
    it("has a grammar for each language it can return", async () => {
        const { bundledLanguages } = await import("shiki/bundle/web");
        const { extraLanguages } = await import("./shiki-highlighter");
        const missing = detectableLanguages().filter(
            (lang) => lang !== "text" && !(lang in bundledLanguages) && !(lang in extraLanguages)
        );
        expect(missing).toEqual([]);
    });

    it.each([
        ["main.rs", "rust", 'fn main() { let x = "a"; }'],
        ["main.go", "go", 'func main() { x := "a" }'],
        ["Cargo.toml", "toml", '[package]\nname = "a"'],
        ["build.ps1", "powershell", '$x = Get-Item "a"'],
        ["Dockerfile", "dockerfile", "FROM node:20\nRUN npm ci"],
        ["Makefile", "makefile", "all: build\n\tcargo build"],
        ["setup.cfg", "ini", "[metadata]\nname = a"],
        [".env", "dotenv", "# comment\nKEY=value"],
        ["App.kt", "kotlin", 'fun main() { val x = "a" }'],
        ["View.swift", "swift", 'func main() { let x = "a" }'],
        ["Program.cs", "csharp", 'class A { string x = "a"; }'],
        ["app.rb", "ruby", 'def a; "b"; end'],
    ])("highlights %s as %s, in colour", async (file, lang, code) => {
        const { codeToHtml } = await import("./shiki-highlighter");
        expect(detectLanguage(file)).toBe(lang);
        const html = await codeToHtml(code, { lang, theme: "github-dark-high-contrast" });
        // A grammar that loaded tokenizes the code into differently coloured spans.
        const colours = new Set([...html.matchAll(/color:(#[0-9A-Fa-f]+)/g)].map((m) => m[1]));
        expect(colours.size).toBeGreaterThan(1);
    });
});

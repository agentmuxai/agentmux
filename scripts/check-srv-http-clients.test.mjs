// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Unit tests for check-srv-http-clients.mjs.

import { describe, expect, it } from "vitest";
import { checkFile, collect } from "./check-srv-http-clients.mjs";

const lines = (path, text) => checkFile(path, text).map((v) => v.line);

describe("check-srv-http-clients", () => {
    it("flags each direct way to build a client, with its line", () => {
        const text = [
            "fn a() {",
            "    let c = reqwest::Client::new();",
            "    let d = reqwest::Client::builder().timeout(t).build();",
            "    let e = reqwest::Client::default();",
            "    let f = reqwest::ClientBuilder::new();",
            "    let g = reqwest::get(url).await;",
            "}",
        ].join("\n");
        expect(lines("crates/srv/src/backend/x.rs", text)).toEqual([2, 3, 4, 5, 6]);
    });

    it("flags the short form only where reqwest's Client is imported", () => {
        const body = "fn a() { let c = Client::builder().build(); let d = Client::new(); }\n";
        expect(lines("crates/srv/src/backend/x.rs", `use reqwest::{Client, Url};\n${body}`)).toEqual([2]);
        expect(lines("crates/srv/src/backend/y.rs", `use reqwest::Client;\n${body}`)).toEqual([2]);
        expect(lines("crates/srv/src/backend/z.rs", `use other::Client;\n${body}`)).toEqual([]);
    });

    it("leaves the helper, test files, test modules and comments alone", () => {
        const direct = "let c = reqwest::Client::new();\n";
        expect(checkFile("crates/srv/src/http_client.rs", direct)).toEqual([]);
        expect(checkFile("crates/srv/src/http_client_tests.rs", direct)).toEqual([]);
        expect(checkFile("crates/srv/src/server/tests.rs", direct)).toEqual([]);
        expect(checkFile("crates/srv/src/muxbus/ws_connect_tests.rs", direct)).toEqual([]);
        expect(checkFile("crates/srv/src/test_support.rs", direct)).toEqual([]);
        expect(checkFile("crates/srv/tests/x.rs", direct)).toEqual([]);
        expect(checkFile("crates/srv/src/backend/x.rs", `fn a() {}\n#[cfg(test)]\nmod tests {\n    fn t() { ${direct} }\n}\n`)).toEqual([]);
        expect(checkFile("crates/srv/src/backend/x.rs", "// built with reqwest::Client::new() once\nlet c = crate::http_client::client(); // not reqwest::Client::new()\n")).toEqual([]);
    });

    it("sees production code after a test module in the middle of a file, and none inside it", () => {
        const text = [
            "fn a() {}",
            "#[cfg(test)]",
            "mod tests {",
            '    fn t() { let s = "}"; let c = reqwest::Client::new(); }',
            "}",
            "fn b() { let c = reqwest::Client::builder(); }",
        ].join("\n");
        expect(lines("crates/mcp/src/heartbeat.rs", text)).toEqual([6]);
    });

    it("flags a constructor passed as a function and ignores strings and test-only items", () => {
        expect(lines("crates/cef/src/x.rs", "CLIENT.get_or_init(reqwest::Client::new)\n")).toEqual([1]);
        expect(lines("crates/cef/src/x.rs", 'let s = "reqwest::Client::new()"; let r = r#"reqwest::Client::new()"#;\n')).toEqual([]);
        expect(lines("crates/srv/src/x.rs", "#[cfg(test)]\nfn helper() -> reqwest::Client {\n    reqwest::Client::new()\n}\nfn prod() { reqwest::Client::new(); }\n")).toEqual([5]);
        expect(lines("crates/srv/src/x.rs", '#[cfg(test)]\n#[path = "x_tests.rs"]\nmod tests;\nfn prod() { reqwest::Client::new(); }\n')).toEqual([4]);
        expect(lines("crates/srv/src/y.rs", "#![cfg(test)]\nfn t() { reqwest::Client::new(); }\n")).toEqual([]);
    });

    it("finds none in the tree today", () => {
        expect(collect()).toEqual([]);
    });

    describe("a helper build that ends in a panic, in srv", () => {
        const chain = (end) => `fn c() -> Client {\n    crate::http_client::builder()\n        .timeout(t)\n        .build()\n        ${end}\n}\n`;

        it("flags unwrap_or_default, expect and unwrap, on the line of the ending", () => {
            for (const end of [".unwrap_or_default()", '.expect("builds")', ".unwrap()"]) {
                expect(lines("crates/srv/src/identity/x.rs", chain(end))).toEqual([5]);
            }
        });

        it("flags the ending when the chain holds a closure with its own statements and when it is on one line", () => {
            const closure = "fn c() {\n    crate::http_client::builder()\n        .redirect(Policy::custom(|a| {\n            let x = 1;\n            a.follow()\n        }))\n        .build()\n        .expect(\"x\")\n}\n";
            expect(lines("crates/srv/src/drone/x.rs", closure)).toEqual([8]);
            expect(lines("crates/srv/src/a.rs", "fn c() { let c = crate::http_client::builder().build().unwrap(); }\n")).toEqual([1]);
        });

        it("leaves the sites that return the error, the finisher and the other crates alone", () => {
            for (const end of [".map_err(|e| e.to_string())?", ".ok()?", ".unwrap_or_else(|_| crate::http_client::client())"]) {
                expect(lines("crates/srv/src/identity/x.rs", chain(end))).toEqual([]);
            }
            expect(lines("crates/srv/src/a.rs", "fn c() {\n    crate::http_client::client_with(|b| b.timeout(t))\n}\n")).toEqual([]);
            expect(lines("crates/mcp/src/main.rs", chain('.expect("http client")').replace("crate::http_client", "agentmux_common::local_http"))).toEqual([]);
        });

        it("does not blame a later build for an earlier builder, a comment or a string", () => {
            const text = 'fn a() { let b = crate::http_client::builder(); let c = b.build(); }\nfn d() { let x = other.build().expect("x"); }\n// crate::http_client::builder().build().unwrap()\nfn e() { let s = "crate::http_client::builder().build().unwrap()"; }\n';
            expect(lines("crates/srv/src/a.rs", text)).toEqual([]);
        });

        it("leaves a test module alone", () => {
            expect(lines("crates/srv/src/a.rs", `fn a() {}\n#[cfg(test)]\nmod tests {\n${chain(".unwrap()")}}\n`)).toEqual([]);
        });
    });
});

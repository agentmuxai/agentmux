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
});

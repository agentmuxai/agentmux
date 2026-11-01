// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The Google client file Connect Google imports: only a Desktop client's, and never the secret in a message.

import { describe, expect, it } from "vitest";
import { MAX_CLIENT_JSON_BYTES, parseGoogleClientJson } from "./oauth-client-json";

const ID = "994249352117-abc.apps.googleusercontent.com";
const SECRET = "GOCSPX-not-a-real-secret";
const desktop = (over: Record<string, unknown> = {}) =>
    JSON.stringify({
        installed: {
            client_id: ID,
            project_id: "agentmux-dev",
            auth_uri: "https://accounts.google.com/o/oauth2/auth",
            token_uri: "https://oauth2.googleapis.com/token",
            client_secret: SECRET,
            redirect_uris: ["http://localhost"],
            ...over,
        },
    });

describe("parseGoogleClientJson", () => {
    it("takes the id and secret from a Desktop client's file", () => {
        expect(parseGoogleClientJson(desktop())).toEqual({ clientId: ID, clientSecret: SECRET });
    });

    it("trims what it takes", () => {
        expect(parseGoogleClientJson(desktop({ client_id: ` ${ID}\n`, client_secret: ` ${SECRET} ` }))).toEqual({
            clientId: ID,
            clientSecret: SECRET,
        });
    });

    it("says a Web client is the wrong kind, and how to make the right one", () => {
        const web = JSON.stringify({ web: { client_id: ID, client_secret: SECRET } });
        const r = parseGoogleClientJson(web);
        expect("error" in r && r.error).toMatch(/Web application client/);
        expect("error" in r && r.error).toMatch(/Desktop app/);
    });

    it("refuses anything else with a plain sentence", () => {
        for (const text of ["", "not json", "[]", "null", "42", "{}", JSON.stringify({ installed: [] }), JSON.stringify({ installed: { client_id: "abc", client_secret: SECRET } })]) {
            const r = parseGoogleClientJson(text);
            expect("error" in r && r.error).toMatch(/isn't a Google OAuth client file/);
        }
    });

    it("asks for a fresh download when the file has no secret", () => {
        const r = parseGoogleClientJson(desktop({ client_secret: "" }));
        expect("error" in r && r.error).toMatch(/no client secret/);
    });

    it("refuses a file too big to be one, without parsing it", () => {
        const r = parseGoogleClientJson(desktop({ pad: "x".repeat(MAX_CLIENT_JSON_BYTES) }));
        expect("error" in r).toBe(true);
    });

    it("never puts the secret in an error", () => {
        for (const text of [desktop({ client_id: "bad" }), JSON.stringify({ web: { client_secret: SECRET } }), desktop({ client_secret: "" }), `{"installed": {"client_secret": "${SECRET}"`]) {
            const r = parseGoogleClientJson(text);
            expect("error" in r && r.error.includes(SECRET)).toBe(false);
        }
    });
});

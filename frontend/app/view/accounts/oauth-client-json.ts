// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The file Google's console downloads for an OAuth client ("Download JSON"): what Connect Google
// takes from it. Only a Desktop-app client is one AgentMux can use (a loopback redirect), and its
// file is `{"installed": {"client_id", "client_secret", ...}}`. A Web client's file says `"web"`.
// The secret is returned to the caller's input and never put in an error, a log or a note.

export interface ParsedClientJson {
    clientId: string;
    clientSecret: string;
}

/** A file bigger than this isn't a client file (the real one is about 400 bytes). */
export const MAX_CLIENT_JSON_BYTES = 16 * 1024;

const WRONG_FILE = "This isn't a Google OAuth client file. In the Cloud console, open the client and use Download JSON.";
const WEB_CLIENT =
    "This is a Web application client. AgentMux needs a Desktop app client: in the Cloud console, Clients → Create client → Application type “Desktop app”, then download its JSON.";

/** The client id and secret in a downloaded Desktop-client JSON, or a plain sentence saying why not. */
export function parseGoogleClientJson(text: string): ParsedClientJson | { error: string } {
    if (text.length > MAX_CLIENT_JSON_BYTES) return { error: WRONG_FILE };
    let doc: unknown;
    try {
        doc = JSON.parse(text);
    } catch {
        return { error: WRONG_FILE };
    }
    if (typeof doc !== "object" || doc === null || Array.isArray(doc)) return { error: WRONG_FILE };
    const root = doc as Record<string, unknown>;
    if ("web" in root && !("installed" in root)) return { error: WEB_CLIENT };
    const installed = root.installed;
    if (typeof installed !== "object" || installed === null || Array.isArray(installed)) return { error: WRONG_FILE };
    const c = installed as Record<string, unknown>;
    const clientId = typeof c.client_id === "string" ? c.client_id.trim() : "";
    const clientSecret = typeof c.client_secret === "string" ? c.client_secret.trim() : "";
    if (!clientId.endsWith(".apps.googleusercontent.com")) return { error: WRONG_FILE };
    if (!clientSecret) return { error: "This client file has no client secret. Download the JSON again from the client's page, or reset the secret there." };
    return { clientId, clientSecret };
}

#!/usr/bin/env node
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Sign AgentMux widget packages (docs/specs/SPEC_WIDGET_SHARING_2026_10_10.md §2).
 *
 *   agentmux-widget keygen <key-file>          a new Ed25519 key; keep it private
 *   agentmux-widget sign <folder> --key <file> writes <folder>/widget.sig
 *   agentmux-widget verify <folder>            checks widget.sig, prints the fingerprint
 *
 * The content hash is AgentMux's own (SPEC_USER_WIDGETS_AND_WIDGET_API §8.2):
 * each file's SHA-256, then SHA-256 over "path\0hash\n" in path order, with
 * widget.sig itself left out. Node's own crypto; no dependencies.
 */

import { createHash, createPrivateKey, createPublicKey, generateKeyPairSync, sign, verify } from "node:crypto";
import { existsSync, lstatSync, readdirSync, readFileSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { pathToFileURL } from "node:url";

export const SIG_FILE = "widget.sig";
const CONTEXT = "agentmux-widget-sig-v1\0";
const MAX_BYTES = 50 * 1024 * 1024;
const MAX_FILES = 2000;
// DER prefixes that wrap a raw 32-byte Ed25519 seed / public key.
const PKCS8_PREFIX = Buffer.from("302e020100300506032b657004220420", "hex");
const SPKI_PREFIX = Buffer.from("302a300506032b6570032100", "hex");

/** relative path → SHA-256 hex of every file in `dir`, as AgentMux hashes it. */
export function hashFiles(dir) {
    const out = new Map();
    let total = 0;
    const walk = (abs, rel) => {
        for (const name of readdirSync(abs)) {
            const p = join(abs, name);
            const r = rel ? `${rel}/${name}` : name;
            const st = lstatSync(p);
            if (st.isSymbolicLink()) throw new Error(`${r} is a link; a package can't contain links`);
            if (st.isDirectory()) {
                walk(p, r);
                continue;
            }
            if (r === SIG_FILE) continue;
            const bytes = readFileSync(p);
            total += bytes.length;
            if (total > MAX_BYTES) throw new Error("the package is over 50 MB");
            out.set(r, createHash("sha256").update(bytes).digest("hex"));
            if (out.size > MAX_FILES) throw new Error(`the package has over ${MAX_FILES} files`);
        }
    };
    walk(dir, "");
    return out;
}

/** The package's content hash. */
export function packageHash(dir) {
    const files = [...hashFiles(dir)].sort(([a], [b]) => Buffer.compare(Buffer.from(a), Buffer.from(b)));
    const h = createHash("sha256");
    for (const [path, fileHash] of files) h.update(path).update("\0").update(fileHash).update("\n");
    return h.digest("hex");
}

export function signedMessage(id, version, hash) {
    return Buffer.from(`${CONTEXT}${id}\0${version}\0${hash}`);
}

/** `K7Q2-MZ4D-PX3A-9TWE`: the first 10 bytes of the key's SHA-256, base32. */
export function fingerprint(publicKeyB64) {
    const digest = createHash("sha256").update(Buffer.from(publicKeyB64, "base64")).digest().subarray(0, 10);
    const alphabet = "ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
    let bits = 0;
    let buf = 0;
    let text = "";
    for (const b of digest) {
        buf = (buf << 8) | b;
        bits += 8;
        while (bits >= 5) {
            bits -= 5;
            text += alphabet[(buf >> bits) & 31];
        }
    }
    if (bits > 0) text += alphabet[(buf << (5 - bits)) & 31];
    return text.match(/.{1,4}/g).join("-");
}

function manifestOf(dir) {
    const m = JSON.parse(readFileSync(join(dir, "widget.json"), "utf8"));
    if (typeof m.id !== "string" || typeof m.version !== "string") throw new Error("widget.json needs an id and a version");
    return m;
}

/** A key from a 32-byte seed (base64). */
export function keyFromSeed(seedB64) {
    const privateKey = createPrivateKey({ key: Buffer.concat([PKCS8_PREFIX, Buffer.from(seedB64, "base64")]), format: "der", type: "pkcs8" });
    const publicKey = createPublicKey(privateKey).export({ format: "der", type: "spki" }).subarray(SPKI_PREFIX.length).toString("base64");
    return { privateKey, publicKey };
}

/** The widget.sig text for the package in `dir`. */
export function signPackage(dir, seedB64) {
    const m = manifestOf(dir);
    const { privateKey, publicKey } = keyFromSeed(seedB64);
    const signature = sign(null, signedMessage(m.id, m.version, packageHash(dir)), privateKey).toString("base64");
    return JSON.stringify({ sigVersion: 1, algorithm: "ed25519", publicKey, signature }, null, 2) + "\n";
}

/** The fingerprint of the key that signed the package in `dir`; throws if it doesn't verify. */
export function verifyPackage(dir) {
    const m = manifestOf(dir);
    const sig = JSON.parse(readFileSync(join(dir, SIG_FILE), "utf8"));
    if (sig.sigVersion !== 1 || sig.algorithm !== "ed25519") throw new Error(`${SIG_FILE} isn't a version 1 ed25519 signature`);
    const key = createPublicKey({ key: Buffer.concat([SPKI_PREFIX, Buffer.from(sig.publicKey, "base64")]), format: "der", type: "spki" });
    if (!verify(null, signedMessage(m.id, m.version, packageHash(dir)), key, Buffer.from(sig.signature, "base64"))) {
        throw new Error("its signature doesn't match its files: sign it again after the last edit");
    }
    return fingerprint(sig.publicKey);
}

function main(argv) {
    const [cmd, target, ...rest] = argv;
    const opt = (name) => {
        const i = rest.indexOf(name);
        return i >= 0 ? rest[i + 1] : undefined;
    };
    if (cmd === "keygen" && target) {
        if (existsSync(target)) throw new Error(`${target} exists; pick another file`);
        const { privateKey } = generateKeyPairSync("ed25519");
        const seed = privateKey.export({ format: "der", type: "pkcs8" }).subarray(PKCS8_PREFIX.length).toString("base64");
        const { publicKey } = keyFromSeed(seed);
        writeFileSync(target, JSON.stringify({ algorithm: "ed25519", seed, publicKey }, null, 2) + "\n", { mode: 0o600 });
        console.log(`Wrote ${target}. Keep it private. Your key's fingerprint: ${fingerprint(publicKey)}`);
        return;
    }
    if (cmd === "sign" && target && opt("--key")) {
        const dir = resolve(target);
        const key = JSON.parse(readFileSync(opt("--key"), "utf8"));
        writeFileSync(join(dir, SIG_FILE), signPackage(dir, key.seed));
        console.log(`Signed ${manifestOf(dir).id} ${manifestOf(dir).version} · ${fingerprint(key.publicKey)}`);
        return;
    }
    if (cmd === "verify" && target) {
        console.log(`${manifestOf(resolve(target)).id}: signed by ${verifyPackage(resolve(target))}`);
        return;
    }
    console.error("usage: agentmux-widget keygen <key-file> | sign <folder> --key <key-file> | verify <folder>");
    process.exitCode = 2;
}

if (import.meta.url === pathToFileURL(process.argv[1] ?? "").href) {
    try {
        main(process.argv.slice(2));
    } catch (e) {
        console.error(e instanceof Error ? e.message : String(e));
        process.exitCode = 1;
    }
}

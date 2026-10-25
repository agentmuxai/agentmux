#!/usr/bin/env node
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The signed manifest of a release's files, made and checked by the release
// workflow (docs/specs/SPEC_SIGNED_UPDATE_CHECKSUMS_2026_10_25.md).
//
//   build  --dir <files> --tag vX.Y.Z --out <dir> [--created-at <iso>]
//       writes AgentMux-<v>-manifest.json (every file in <files>, with its size
//       and SHA-256) and AgentMux-<v>-envelope.json (the release's tag and the
//       manifest's SHA-256: the small document that is signed) into <dir>.
//   sign-kms --envelope <file> --key-id <alias|arn> --id <k1> --out <file>
//       signs the envelope's exact bytes with an Ed25519 key held by a signing
//       service (`aws kms sign`, raw message), and writes the signature line
//       `<id> <base64>` (AgentMux-<v>-envelope.json.sig).
//   sign-key --envelope <file> --private-key <pem> --id <k1> --out <file>
//       the same with a local key; for tests, never for a release.
//   verify --dir <dir> --tag vX.Y.Z --id <k1> [--files <files>]
//          (--public-key <base64 of the raw 32 bytes> | --keys-file <json>)
//       checks what the app checks (crates/common/src/update_manifest.rs): the
//       signature of the envelope, its tag, the manifest's hash and format; with
//       --files, also that every file in <files> is in the manifest with that
//       size and hash, and that the manifest lists nothing else. --keys-file
//       is crates/common/update-keys.json, the keys the app pins.
//
//   self-check --message <file> --signature <file with base64> --public-key <base64>
//       checks one plain Ed25519 signature of a file's bytes with one raw public
//       key: the first thing to try with a new signing key, before anything
//       depends on it. Prints `verifies`, or fails.
//
// Exits non-zero, with the reason on stderr, on any failure: a release that
// can't be signed and checked is not published.

import { createHash, createPrivateKey, createPublicKey, sign as cryptoSign, verify as cryptoVerify } from "node:crypto";
import { execFileSync } from "node:child_process";
import { readdirSync, readFileSync, statSync, writeFileSync, mkdirSync } from "node:fs";
import { join } from "node:path";
import { pathToFileURL } from "node:url";

export const ENVELOPE_FORMAT = 1;
export const MANIFEST_FORMAT = 1;
export const MAX_ENVELOPE_BYTES = 4096;
export const MAX_MANIFEST_BYTES = 1024 * 1024;

const SPKI_ED25519_PREFIX = Buffer.from("302a300506032b6570032100", "hex");
const SHA256_RE = /^[0-9a-f]{64}$/;
// The same rule as `is_plain_file_name` in crates/common/src/update_manifest.rs.
const isPlainName = (n) => typeof n === "string" && /^[A-Za-z0-9._+-]{1,200}$/.test(n) && !n.startsWith(".") && !n.includes("..");

export const sha256Hex = (bytes) => createHash("sha256").update(bytes).digest("hex");

/** The version in a tag: `v0.60.1` -> `0.60.1`. */
export function versionOfTag(tag) {
    const m = /^v(\d+\.\d+\.\d+)$/.exec(tag);
    if (!m) throw new Error(`${JSON.stringify(tag)} isn't a release tag (vX.Y.Z)`);
    return m[1];
}

/** Whether `signatureB64` is a plain Ed25519 signature of `message` by the raw 32-byte key. */
export function selfCheck({ message, signatureB64, publicKeyB64 }) {
    const raw = Buffer.from(publicKeyB64, "base64");
    if (raw.length !== 32) throw new Error("the public key isn't 32 raw bytes in base64");
    const sig = Buffer.from(String(signatureB64).trim(), "base64");
    if (sig.length !== 64 || !cryptoVerify(null, Buffer.from(message), publicKeyOf(raw), sig)) {
        throw new Error("the signature doesn't verify as plain Ed25519");
    }
}

export const manifestName = (tag) => `AgentMux-${versionOfTag(tag)}-manifest.json`;
export const envelopeName = (tag) => `AgentMux-${versionOfTag(tag)}-envelope.json`;
export const signatureName = (tag) => `${envelopeName(tag)}.sig`;

/** Every file in `dir`, with its size and SHA-256, by name. */
export function listFiles(dir) {
    const files = [];
    for (const name of readdirSync(dir).sort()) {
        const path = join(dir, name);
        if (!statSync(path).isFile()) continue;
        if (!isPlainName(name)) throw new Error(`${JSON.stringify(name)} isn't a plain release file name`);
        const bytes = readFileSync(path);
        if (bytes.length === 0) throw new Error(`${name} is empty`);
        files.push({ name, size: bytes.length, sha256: sha256Hex(bytes) });
    }
    if (files.length === 0) throw new Error(`no files in ${dir}`);
    return files;
}

/** The manifest and envelope texts for a release's files. */
export function buildRelease({ files, tag, createdAt }) {
    const manifest = JSON.stringify({ format: MANIFEST_FORMAT, version: versionOfTag(tag), tag, files }, null, 2) + "\n";
    if (Buffer.byteLength(manifest) > MAX_MANIFEST_BYTES) throw new Error("the manifest is too large");
    const envelope =
        JSON.stringify(
            {
                format: ENVELOPE_FORMAT,
                tag,
                manifest_sha256: sha256Hex(manifest),
                created_at: createdAt ?? new Date().toISOString().replace(/\.\d+Z$/, "Z"),
            },
            null,
            2,
        ) + "\n";
    if (Buffer.byteLength(envelope) > MAX_ENVELOPE_BYTES) throw new Error("the envelope is too large to sign");
    return { manifest, envelope };
}

const publicKeyOf = (raw) => createPublicKey({ key: Buffer.concat([SPKI_ED25519_PREFIX, raw]), format: "der", type: "spki" });

/** The signature line for `envelope` made with a local Ed25519 private key (PEM). Tests only. */
export function signWithKey(envelope, privateKeyPem, id) {
    return `${id} ${cryptoSign(null, Buffer.from(envelope), createPrivateKey(privateKeyPem)).toString("base64")}\n`;
}

/** The signature line for the envelope's file, made by the signing service. */
export function signWithKms(envelopePath, keyId, id) {
    const out = execFileSync(
        "aws",
        [
            "kms", "sign",
            "--key-id", keyId,
            "--message", `fileb://${envelopePath}`,
            "--message-type", "RAW",
            "--signing-algorithm", "ED25519_SHA_512",
            "--query", "Signature",
            "--output", "text",
        ],
        { encoding: "utf8" },
    ).trim();
    if (!/^[A-Za-z0-9+/=]{80,100}$/.test(out)) throw new Error("the signing service didn't return a signature");
    return `${id} ${out}\n`;
}

/**
 * What the app checks. Throws with the reason; returns the parsed manifest.
 * `files` (optional): the release's real files, `{name, size, sha256}` each.
 */
export function verifyRelease({ envelope, signatures, manifest, tag, publicKeyB64, id, files }) {
    const raw = Buffer.from(publicKeyB64, "base64");
    if (raw.length !== 32) throw new Error("the public key isn't 32 raw bytes in base64");
    if (Buffer.byteLength(envelope) > MAX_ENVELOPE_BYTES) throw new Error("the envelope is too large");
    const line = String(signatures)
        .split(/\r?\n/)
        .map((l) => l.trim().split(/\s+/))
        .find((p) => p.length === 2 && p[0] === id);
    if (!line) throw new Error(`no signature line for key ${id}`);
    const sig = Buffer.from(line[1], "base64");
    if (sig.length !== 64 || !cryptoVerify(null, Buffer.from(envelope), publicKeyOf(raw), sig)) {
        throw new Error("the signature doesn't match the envelope");
    }
    const env = JSON.parse(envelope);
    if (env.format !== ENVELOPE_FORMAT) throw new Error(`envelope format ${env.format} isn't understood`);
    if (env.tag !== tag) throw new Error(`the envelope is for ${env.tag}, not ${tag}`);
    if (!SHA256_RE.test(env.manifest_sha256)) throw new Error("manifest_sha256 isn't a SHA-256");
    if (Buffer.byteLength(manifest) > MAX_MANIFEST_BYTES) throw new Error("the manifest is too large");
    if (sha256Hex(manifest) !== env.manifest_sha256) throw new Error("the manifest isn't the one the signature covers");
    const m = JSON.parse(manifest);
    if (m.format !== MANIFEST_FORMAT) throw new Error(`manifest format ${m.format} isn't understood`);
    if (m.tag !== env.tag || m.version !== versionOfTag(env.tag)) throw new Error("the manifest names another release than the envelope");
    if (!Array.isArray(m.files) || m.files.length === 0) throw new Error("the manifest lists no files");
    const seen = new Set();
    for (const f of m.files) {
        if (!isPlainName(f.name)) throw new Error(`${JSON.stringify(f.name)} isn't a plain file name`);
        if (!SHA256_RE.test(f.sha256)) throw new Error(`${f.name}: sha256 isn't a SHA-256`);
        if (!Number.isInteger(f.size) || f.size <= 0) throw new Error(`${f.name}: size isn't positive`);
        if (seen.has(f.name)) throw new Error(`${f.name} is listed twice`);
        seen.add(f.name);
    }
    if (files) {
        const real = new Map(files.map((f) => [f.name, f]));
        for (const f of m.files) {
            const r = real.get(f.name);
            if (!r) throw new Error(`${f.name} is in the manifest but isn't among the files`);
            if (r.size !== f.size || r.sha256 !== f.sha256) throw new Error(`${f.name} isn't the file the manifest describes`);
        }
        for (const r of real.values()) if (!seen.has(r.name)) throw new Error(`${r.name} isn't in the manifest`);
    }
    return m;
}

function parseArgs(argv) {
    const out = {};
    for (let i = 0; i < argv.length; i++) {
        if (!argv[i].startsWith("--")) throw new Error(`unexpected argument ${argv[i]}`);
        const key = argv[i].slice(2);
        if (i + 1 >= argv.length) throw new Error(`--${key} needs a value`);
        out[key] = argv[++i];
    }
    return out;
}

const need = (args, ...keys) => {
    for (const k of keys) if (!args[k]) throw new Error(`--${k} is required`);
};

function main(argv) {
    const [command, ...rest] = argv;
    const args = parseArgs(rest);
    switch (command) {
        case "build": {
            need(args, "dir", "tag", "out");
            const { manifest, envelope } = buildRelease({ files: listFiles(args.dir), tag: args.tag, createdAt: args["created-at"] });
            mkdirSync(args.out, { recursive: true });
            writeFileSync(join(args.out, manifestName(args.tag)), manifest);
            writeFileSync(join(args.out, envelopeName(args.tag)), envelope);
            console.log(`${manifestName(args.tag)}, ${envelopeName(args.tag)} written to ${args.out}`);
            return;
        }
        case "sign-kms": {
            need(args, "envelope", "key-id", "id", "out");
            writeFileSync(args.out, signWithKms(args.envelope, args["key-id"], args.id));
            console.log(`signed ${args.envelope} as ${args.id}`);
            return;
        }
        case "sign-key": {
            need(args, "envelope", "private-key", "id", "out");
            writeFileSync(args.out, signWithKey(readFileSync(args.envelope), readFileSync(args["private-key"], "utf8"), args.id));
            console.log(`signed ${args.envelope} as ${args.id} (local key: not for a release)`);
            return;
        }
        case "verify": {
            need(args, "dir", "tag", "id");
            const read = (name) => readFileSync(join(args.dir, name));
            let publicKey = args["public-key"];
            if (!publicKey) {
                need(args, "keys-file");
                publicKey = JSON.parse(readFileSync(args["keys-file"], "utf8")).keys.find((k) => k.id === args.id)?.public_key;
                if (!publicKey) throw new Error(`the keys file has no key ${args.id}`);
            }
            const m = verifyRelease({
                envelope: read(envelopeName(args.tag)),
                signatures: read(signatureName(args.tag)).toString("utf8"),
                manifest: read(manifestName(args.tag)),
                tag: args.tag,
                publicKeyB64: publicKey,
                id: args.id,
                files: args.files ? listFiles(args.files) : undefined,
            });
            console.log(`verified ${args.tag}: ${m.files.length} files`);
            return;
        }
        case "self-check": {
            need(args, "message", "signature", "public-key");
            selfCheck({
                message: readFileSync(args.message),
                signatureB64: readFileSync(args.signature, "utf8"),
                publicKeyB64: args["public-key"],
            });
            console.log("verifies");
            return;
        }
        default:
            throw new Error(`usage: release-manifest.mjs <build|sign-kms|sign-key|verify|self-check> --option value ...`);
    }
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
    try {
        main(process.argv.slice(2));
    } catch (e) {
        console.error(`release-manifest: ${e.message}`);
        process.exit(1);
    }
}

#!/usr/bin/env node
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The signed manifest of a release's files, made and checked by the release
// workflow (docs/specs/SPEC_SIGNED_UPDATE_CHECKSUMS_2026_10_25.md).
//
//   build  --dir <files> --tag vX.Y.Z --out <dir> [--created-at <iso>]
//          [--image <name>=<registry path>@sha256:<64 hex>]...
//       writes AgentMux-<v>-manifest.json (every file in <files>, with its size
//       and SHA-256; with --image, also the container images the release
//       published, each by the digest of the pushed image) and
//       AgentMux-<v>-envelope.json (the release's tag and the manifest's
//       SHA-256: the small document that is signed) into <dir>.
//   sign-kms --envelope <file> --key-id <alias|arn> --id <k1> --out <file>
//          [--sshsig-out <file>] [--keys-file <json>]
//       signs the envelope's exact bytes with an Ed25519 key held by a signing
//       service (`aws kms sign`, raw message), and writes the signature line
//       `<id> <base64>` (AgentMux-<v>-envelope.json.sig). It then signs the same
//       envelope a second time with the same key, in SSH signature form
//       (SSHSIG), for systems that can check that and not the line above (a
//       stock Mac), and writes AgentMux-<v>-envelope.json.sshsig next to --out
//       (or to --sshsig-out). The signature is checked against the pinned key
//       (--keys-file, default crates/common/update-keys.json) before it is
//       written. docs/specs/SPEC_RELEASE_SSHSIG_SECOND_SIGNATURE_2026_11_15.md.
//   sign-key --envelope <file> --private-key <pem> --id <k1> --out <file>
//          [--sshsig-out <file>]
//       the same with a local key; for tests, never for a release.
//   verify --dir <dir> --tag vX.Y.Z --id <k1> [--files <files>]
//          (--public-key <base64 of the raw 32 bytes> | --keys-file <json>)
//       checks what the app checks (crates/common/src/update_manifest.rs): the
//       signature of the envelope, its tag, the manifest's hash and format; with
//       --files, also that every file in <files> is in the manifest with that
//       size and hash, and that the manifest lists nothing else. --keys-file
//       is crates/common/update-keys.json, the keys the app pins. The SSH
//       signature (.sshsig) is checked when it is there, and must be there for
//       a release from SSHSIG_FLOOR on: with this script's own parser and with
//       `ssh-keygen -Y verify` against the pinned key (--no-ssh-keygen skips
//       the second, for a machine that has none: tests only).
//
//   self-check --message <file> --signature <file with base64> --public-key <base64>
//       checks one plain Ed25519 signature of a file's bytes with one raw public
//       key: the first thing to try with a new signing key, before anything
//       depends on it. Prints `verifies`, or fails.
//
// Exits non-zero, with the reason on stderr, on any failure: a release that
// can't be signed and checked is not published.

import { createHash, createPrivateKey, createPublicKey, sign as cryptoSign, verify as cryptoVerify } from "node:crypto";
import { execFileSync, spawnSync } from "node:child_process";
import { existsSync, mkdtempSync, readdirSync, readFileSync, renameSync, rmSync, statSync, writeFileSync, mkdirSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

export const ENVELOPE_FORMAT = 1;
/** The namespace every release SSH signature is made and checked under. Fixed: install.sh and install.ps1 use the same. */
export const SSHSIG_NAMESPACE = "agentmux-release";
/** The first release that carries a .sshsig. From it on, a release without one is refused, and so is a script that falls back. install.sh has the same value (a test keeps them equal). */
export const SSHSIG_FLOOR = "0.60.31";
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
export const sshsigName = (tag) => `${envelopeName(tag)}.sshsig`;

/** Whether version `a` is at or above `b`. Both must be exactly `X.Y.Z` (digits): anything else throws, so a suffix can never compare as "below" through NaN. */
export function versionAtLeast(a, b) {
    for (const v of [a, b]) if (!/^\d+\.\d+\.\d+$/.test(String(v))) throw new Error(`${JSON.stringify(v)} isn't a version of digits X.Y.Z`);
    const pa = a.split(".").map(Number);
    const pb = b.split(".").map(Number);
    for (let i = 0; i < 3; i++) if (pa[i] !== pb[i]) return pa[i] > pb[i];
    return true;
}

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

// The images a release names (SPEC_AGENT_IMAGE_DIGEST_IN_SIGNED_MANIFEST). The app
// (crates/common/src/update_manifest.rs) enforces the same rules.
export const MAX_IMAGES = 16;
const IMAGE_NAME_RE = /^[a-z0-9][a-z0-9._-]{0,63}$/;
const IMAGE_REF_HOST_RE = /^[a-z0-9._-]+(?::\d+)?$/;
const IMAGE_REF_COMPONENT_RE = /^[a-z0-9._-]+$/;
const IMAGE_DIGEST_RE = /^sha256:[0-9a-f]{64}$/;

/** `name=host/path@sha256:<hex>` -> `{ name, ref, digest }`. */
export function parseImageArg(text) {
    const m = /^([^=@]+)=([^@]+)@(.+)$/.exec(text ?? "");
    if (!m) throw new Error(`--image ${JSON.stringify(text)} isn't <name>=<registry path>@sha256:<64 hex>`);
    return { name: m[1], ref: m[2], digest: m[3] };
}

/** Throws unless `images` is a list of well-formed, distinct image entries. */
export function checkImages(images) {
    if (!Array.isArray(images)) throw new Error("images isn't a list");
    if (images.length > MAX_IMAGES) throw new Error(`the manifest lists more than ${MAX_IMAGES} images`);
    const seen = new Set();
    for (const img of images) {
        const { name, ref, digest } = img ?? {};
        if (typeof name !== "string" || !IMAGE_NAME_RE.test(name)) throw new Error(`${JSON.stringify(name)} isn't a plain image name`);
        if (typeof ref !== "string" || ref.length > 255) throw new Error(`${name}: ref isn't a registry path`);
        // host[:port]/path/...: no scheme, tag or digest, lowercase components, none empty.
        const parts = ref.split("/");
        if (parts.length < 2 || !IMAGE_REF_HOST_RE.test(parts[0]) || !parts.slice(1).every((c) => IMAGE_REF_COMPONENT_RE.test(c))) {
            throw new Error(`${name}: ${JSON.stringify(ref)} isn't a registry repository path (host/path, no scheme, tag or digest)`);
        }
        if (typeof digest !== "string" || !IMAGE_DIGEST_RE.test(digest)) throw new Error(`${name}: digest isn't sha256:<64 lowercase hex>`);
        if (seen.has(name)) throw new Error(`image ${name} is listed twice`);
        seen.add(name);
    }
}

/** The manifest and envelope texts for a release's files (and images, when it published any). */
export function buildRelease({ files, tag, createdAt, images }) {
    const body = { format: MANIFEST_FORMAT, version: versionOfTag(tag), tag, files };
    if (images?.length) {
        checkImages(images);
        body.images = images.map(({ name, ref, digest }) => ({ name, ref, digest }));
    }
    const manifest = JSON.stringify(body, null, 2) + "\n";
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

/** The environment variable a test names its stand-in signer with: a JSON array, the program and its leading arguments. */
export const SIGNER_ENV = "AGENTMUX_RELEASE_SIGNER";

/**
 * The command that signs. Four cases, by whether a test runner is running (`VITEST`) and whether `AGENTMUX_RELEASE_SIGNER`
 * is set to something:
 * - neither: `aws` from the PATH, as the release workflow runs it;
 * - a test runner and no stand-in: refused, so a test can never reach the real signing service;
 * - a test runner and a stand-in: the stand-in, a JSON array (the program, then any leading arguments: `[process.execPath,
 *   "<its script>"]` runs the same on every system);
 * - a stand-in and no test runner: refused, so a release job never runs a program the environment names.
 */
export function signerCommand(env = process.env) {
    const named = env[SIGNER_ENV];
    const testing = Boolean(env.VITEST);
    if (named === undefined || named === "") {
        if (testing) throw new Error(`refusing to call the real signing service (aws kms sign) from a test: name a stand-in in ${SIGNER_ENV}`);
        return ["aws"];
    }
    if (!testing) throw new Error(`${SIGNER_ENV} is for tests: it is honoured only under a test runner, and nothing was run`);
    let command;
    try {
        command = JSON.parse(named);
    } catch {
        throw new Error(`${SIGNER_ENV} isn't a JSON array of strings`);
    }
    if (!Array.isArray(command) || command.length === 0 || !command.every((a) => typeof a === "string" && a !== "")) throw new Error(`${SIGNER_ENV} isn't a JSON array of strings`);
    return command;
}

/** `aws kms sign` of the file's bytes with the release key, as the base64 the service answers with. */
function kmsSign(messageFile, keyId) {
    const [program, ...leading] = signerCommand();
    const out = execFileSync(
        program,
        [
            ...leading,
            "kms", "sign",
            "--key-id", keyId,
            "--message", `fileb://${messageFile}`,
            "--message-type", "RAW",
            "--signing-algorithm", "ED25519_SHA_512",
            "--query", "Signature",
            "--output", "text",
        ],
        { encoding: "utf8" },
    ).trim();
    if (!/^[A-Za-z0-9+/=]{80,100}$/.test(out)) throw new Error("the signing service didn't return a signature");
    return out;
}

/** The signature line for the envelope's file, made by the signing service. */
export function signWithKms(envelopePath, keyId, id) {
    return `${id} ${kmsSign(envelopePath, keyId)}\n`;
}

// ---- the second signature: SSH signature (SSHSIG) form -------------------------------------------------------------------
// The same Ed25519 key signs a small structure (the "preimage") that holds the SHA-512 of the envelope, so the system's
// `ssh-keygen -Y verify` can check it where openssl can't (a stock Mac). The format is OpenSSH's PROTOCOL.sshsig.

const u32 = (n) => {
    const b = Buffer.alloc(4);
    b.writeUInt32BE(n);
    return b;
};
const sshString = (bytes) => Buffer.concat([u32(bytes.length), Buffer.from(bytes)]);
const SSH_ED25519 = Buffer.from("ssh-ed25519");
const MAX_SSHSIG_BYTES = 2048;

/** The bytes the signer signs for `envelope`: "SSHSIG", the namespace, a reserved empty string, the hash name and the hash. */
export function sshsigPreimage(envelope, namespace = SSHSIG_NAMESPACE) {
    return Buffer.concat([
        Buffer.from("SSHSIG"),
        sshString(Buffer.from(namespace)),
        sshString(Buffer.alloc(0)),
        sshString(Buffer.from("sha512")),
        sshString(createHash("sha512").update(Buffer.from(envelope)).digest()),
    ]);
}

/** The public key as an SSH key blob: string "ssh-ed25519", string(the 32 raw bytes). */
export const sshPublicKeyBlob = (raw) => Buffer.concat([sshString(SSH_ED25519), sshString(raw)]);

/** The armoured SSH signature file for a 64-byte Ed25519 `signature` of the preimage, made by the key `publicKeyRaw`. */
export function sshsigArmor(publicKeyRaw, signature, namespace = SSHSIG_NAMESPACE) {
    if (publicKeyRaw.length !== 32 || signature.length !== 64) throw new Error("an Ed25519 key is 32 bytes and its signature 64");
    const blob = Buffer.concat([
        Buffer.from("SSHSIG"),
        u32(1),
        sshString(sshPublicKeyBlob(publicKeyRaw)),
        sshString(Buffer.from(namespace)),
        sshString(Buffer.alloc(0)),
        sshString(Buffer.from("sha512")),
        sshString(Buffer.concat([sshString(SSH_ED25519), sshString(signature)])),
    ]);
    const b64 = blob.toString("base64");
    const lines = b64.match(/.{1,70}/g) ?? [];
    return `-----BEGIN SSH SIGNATURE-----\n${lines.join("\n")}\n-----END SSH SIGNATURE-----\n`;
}

/** Take an armoured SSH signature apart, strictly: anything unexpected is an error. */
export function parseSshsig(text) {
    if (Buffer.byteLength(text) > MAX_SSHSIG_BYTES) throw new Error("the SSH signature is too large");
    const m = /^-----BEGIN SSH SIGNATURE-----\n([A-Za-z0-9+/=\n]+)\n-----END SSH SIGNATURE-----\n?$/.exec(String(text).replace(/\r\n/g, "\n"));
    if (!m) throw new Error("the SSH signature isn't in the armoured form");
    const buf = Buffer.from(m[1].replace(/\n/g, ""), "base64");
    let at = 0;
    const take = (n) => {
        if (at + n > buf.length) throw new Error("the SSH signature is cut short");
        const out = buf.subarray(at, at + n);
        at += n;
        return out;
    };
    const str = () => take(take(4).readUInt32BE());
    if (take(6).toString() !== "SSHSIG") throw new Error("the SSH signature has no SSHSIG magic");
    if (take(4).readUInt32BE() !== 1) throw new Error("the SSH signature's version isn't 1");
    const keyBlob = str();
    const namespace = str().toString();
    const reserved = str();
    const hashAlg = str().toString();
    const sigBlob = str();
    if (at !== buf.length) throw new Error("the SSH signature has bytes after its end");
    if (reserved.length !== 0) throw new Error("the SSH signature's reserved field isn't empty");
    // The key and the signature, each "ssh-ed25519" and 32 or 64 bytes.
    let k = 0;
    const kstr = (b, n) => {
        const len = b.readUInt32BE(n);
        return [b.subarray(n + 4, n + 4 + len), n + 4 + len];
    };
    const [keyType, k1] = kstr(keyBlob, k);
    const [publicKey, k2] = kstr(keyBlob, k1);
    if (keyType.toString() !== "ssh-ed25519" || publicKey.length !== 32 || k2 !== keyBlob.length) throw new Error("the SSH signature's key isn't an Ed25519 key");
    const [sigType, s1] = kstr(sigBlob, 0);
    const [signature, s2] = kstr(sigBlob, s1);
    if (sigType.toString() !== "ssh-ed25519" || signature.length !== 64 || s2 !== sigBlob.length) throw new Error("the SSH signature isn't an Ed25519 signature");
    return { publicKey, namespace, hashAlg, signature };
}

/**
 * Checks `sshsig` (the armoured text) as a signature of `envelope` by the pinned key (`publicKeyB64`, the 32 raw bytes in
 * base64): the namespace and the hash are the fixed ones, the key INSIDE the signature is the pinned key (it is never
 * trusted on its own), and the Ed25519 signature of the preimage is good. Throws with the reason.
 */
export function verifySshsig({ envelope, sshsig, publicKeyB64 }) {
    const pinned = Buffer.from(publicKeyB64, "base64");
    if (pinned.length !== 32) throw new Error("the public key isn't 32 raw bytes in base64");
    const p = parseSshsig(sshsig);
    if (p.namespace !== SSHSIG_NAMESPACE) throw new Error(`the SSH signature is for the namespace ${JSON.stringify(p.namespace)}, not ${SSHSIG_NAMESPACE}`);
    if (p.hashAlg !== "sha512") throw new Error(`the SSH signature hashes with ${p.hashAlg}, not sha512`);
    if (!p.publicKey.equals(pinned)) throw new Error("the SSH signature was made by a key that isn't the pinned one");
    if (!cryptoVerify(null, sshsigPreimage(envelope), publicKeyOf(pinned), p.signature)) throw new Error("the SSH signature doesn't match the envelope");
}

/** The armoured SSH signature of `envelope` made with a local Ed25519 private key (PEM) whose public half is `publicKeyB64`. Tests only. */
export function signSshsigWithKey(envelope, privateKeyPem, publicKeyB64) {
    const sig = cryptoSign(null, sshsigPreimage(envelope), createPrivateKey(privateKeyPem));
    return sshsigArmor(Buffer.from(publicKeyB64, "base64"), sig);
}

/** The armoured SSH signature of the envelope's file, made by the signing service: the same key, a second message. */
export function signSshsigWithKms(envelopePath, keyId, publicKeyB64) {
    const dir = mkdtempSync(join(tmpdir(), "sshsig-"));
    try {
        const file = join(dir, "preimage");
        writeFileSync(file, sshsigPreimage(readFileSync(envelopePath)));
        const out = kmsSign(file, keyId);
        return sshsigArmor(Buffer.from(publicKeyB64, "base64"), Buffer.from(out, "base64"));
    } finally {
        rmSync(dir, { recursive: true, force: true });
    }
}

/** `ssh-keygen -Y verify` of the envelope against an allowed-signers file that holds only the pinned key. Throws with its words. */
export function sshKeygenVerify({ envelope, sshsig, publicKeyB64, keygen = "ssh-keygen" }) {
    const dir = mkdtempSync(join(tmpdir(), "sshsig-"));
    try {
        const signers = join(dir, "allowed_signers");
        const sigFile = join(dir, "envelope.sshsig");
        writeFileSync(signers, `${SSHSIG_NAMESPACE} namespaces="${SSHSIG_NAMESPACE}" ssh-ed25519 ${sshPublicKeyBlob(Buffer.from(publicKeyB64, "base64")).toString("base64")}\n`);
        writeFileSync(sigFile, sshsig);
        const r = spawnSync(keygen, ["-Y", "verify", "-f", signers, "-I", SSHSIG_NAMESPACE, "-n", SSHSIG_NAMESPACE, "-s", sigFile], { input: Buffer.from(envelope), encoding: "utf8" });
        if (r.error) throw new Error(`ssh-keygen could not be run (${r.error.code ?? r.error.message}): the release job needs the OpenSSH client`);
        if (r.status !== 0) throw new Error(`ssh-keygen -Y verify refused the SSH signature: ${`${r.stdout}${r.stderr}`.trim()}`);
    } finally {
        rmSync(dir, { recursive: true, force: true });
    }
}

/**
 * What the app checks. Throws with the reason; returns the parsed manifest.
 * `files` (optional): the release's real files, `{name, size, sha256}` each.
 */
export function verifyRelease({ envelope, signatures, manifest, tag, publicKeyB64, id, files, sshsig }) {
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
    // The second signature: required from SSHSIG_FLOOR on, checked whenever it is there.
    if (sshsig === undefined) {
        if (versionAtLeast(versionOfTag(tag), SSHSIG_FLOOR)) throw new Error(`${tag} must carry ${sshsigName(tag)} (every release from ${SSHSIG_FLOOR} on does)`);
    } else {
        verifySshsig({ envelope, sshsig, publicKeyB64 });
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
    if (m.images !== undefined) checkImages(m.images);
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

const defaultKeysFile = join(dirname(fileURLToPath(import.meta.url)), "..", "crates", "common", "update-keys.json");

/** The base64 raw public key pinned for `id` in the app's keys file. */
function pinnedKey(keysFile, id) {
    const key = JSON.parse(readFileSync(keysFile, "utf8")).keys.find((k) => k.id === id)?.public_key;
    if (!key) throw new Error(`the keys file has no key ${id}`);
    return key;
}

/** Where the SSH signature goes for a signature line written to `out`: `...envelope.json.sig` -> `...envelope.json.sshsig`. */
export function sshsigPathFor(out) {
    if (!out.endsWith(".sig")) throw new Error("pass --sshsig-out: --out doesn't end in .sig, so the SSH signature's path can't be derived from it");
    return `${out.slice(0, -".sig".length)}.sshsig`;
}

function parseArgs(argv) {
    const out = {};
    for (let i = 0; i < argv.length; i++) {
        if (!argv[i].startsWith("--")) throw new Error(`unexpected argument ${argv[i]}`);
        const key = argv[i].slice(2);
        if (key === "no-ssh-keygen") {
            out[key] = "1";
            continue;
        }
        if (i + 1 >= argv.length) throw new Error(`--${key} needs a value`);
        // --image may be repeated; every other option is given once.
        if (key === "image") (out.images ??= []).push(argv[++i]);
        else out[key] = argv[++i];
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
            const { manifest, envelope } = buildRelease({
                files: listFiles(args.dir),
                tag: args.tag,
                createdAt: args["created-at"],
                images: (args.images ?? []).map(parseImageArg),
            });
            mkdirSync(args.out, { recursive: true });
            writeFileSync(join(args.out, manifestName(args.tag)), manifest);
            writeFileSync(join(args.out, envelopeName(args.tag)), envelope);
            console.log(`${manifestName(args.tag)}, ${envelopeName(args.tag)} written to ${args.out}`);
            return;
        }
        case "sign-kms": {
            need(args, "envelope", "key-id", "id", "out");
            const publicKey = pinnedKey(args["keys-file"] ?? defaultKeysFile, args.id);
            const sshsigOut = args["sshsig-out"] ?? sshsigPathFor(args.out);
            // A file left by an earlier run is never kept: after this command there is the one it made and checked, or none.
            rmSync(sshsigOut, { force: true });
            writeFileSync(args.out, signWithKms(args.envelope, args["key-id"], args.id));
            // The second signature, by the same key (a second message to the same service), checked against the pinned key before it is written (to a temporary name, then renamed: never a partial file).
            const sshsig = signSshsigWithKms(args.envelope, args["key-id"], publicKey);
            verifySshsig({ envelope: readFileSync(args.envelope), sshsig, publicKeyB64: publicKey });
            writeFileSync(`${sshsigOut}.tmp`, sshsig);
            renameSync(`${sshsigOut}.tmp`, sshsigOut);
            console.log(`signed ${args.envelope} as ${args.id}, and in SSH signature form (${sshsigOut})`);
            return;
        }
        case "sign-key": {
            need(args, "envelope", "private-key", "id", "out");
            const pem = readFileSync(args["private-key"], "utf8");
            writeFileSync(args.out, signWithKey(readFileSync(args.envelope), pem, args.id));
            if (args["sshsig-out"]) {
                const publicKey = createPublicKey(pem).export({ format: "der", type: "spki" }).subarray(SPKI_ED25519_PREFIX.length).toString("base64");
                writeFileSync(args["sshsig-out"], signSshsigWithKey(readFileSync(args.envelope), pem, publicKey));
            }
            console.log(`signed ${args.envelope} as ${args.id} (local key: not for a release)`);
            return;
        }
        case "verify": {
            need(args, "dir", "tag", "id");
            const read = (name) => readFileSync(join(args.dir, name));
            let publicKey = args["public-key"];
            if (!publicKey) {
                need(args, "keys-file");
                publicKey = pinnedKey(args["keys-file"], args.id);
            }
            const sshsigFile = join(args.dir, sshsigName(args.tag));
            const sshsig = existsSync(sshsigFile) ? readFileSync(sshsigFile, "utf8") : undefined;
            const m = verifyRelease({
                envelope: read(envelopeName(args.tag)),
                signatures: read(signatureName(args.tag)).toString("utf8"),
                manifest: read(manifestName(args.tag)),
                tag: args.tag,
                publicKeyB64: publicKey,
                id: args.id,
                files: args.files ? listFiles(args.files) : undefined,
                sshsig,
            });
            // The real tool too, on the runner: what a person's system will run against what the release published.
            if (sshsig !== undefined && args["no-ssh-keygen"] === undefined) {
                sshKeygenVerify({ envelope: read(envelopeName(args.tag)), sshsig, publicKeyB64: publicKey });
            }
            console.log(`verified ${args.tag}: ${m.files.length} files${m.images?.length ? `, ${m.images.length} image(s)` : ""}${sshsig !== undefined ? ", SSH signature" : ""}`);
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

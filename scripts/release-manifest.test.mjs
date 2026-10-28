// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Unit tests for release-manifest.mjs (docs/specs/SPEC_SIGNED_UPDATE_CHECKSUMS_2026_10_25.md).
// The fixture release in crates/common/src/update_manifest.rs is made with the
// same fixed key, so the two sides are held to one format.

import { createPrivateKey, createPublicKey } from "node:crypto";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import {
    buildRelease,
    checkImages,
    envelopeName,
    listFiles,
    manifestName,
    parseImageArg,
    selfCheck,
    sha256Hex,
    signWithKey,
    signatureName,
    verifyRelease,
    versionOfTag,
} from "./release-manifest.mjs";

const TAG = "v0.60.1";
const privateKey = (seed) =>
    createPrivateKey({
        key: Buffer.concat([Buffer.from("302e020100300506032b657004220420", "hex"), Buffer.alloc(32, seed)]),
        format: "der",
        type: "pkcs8",
    });
const pemOf = (seed) => privateKey(seed).export({ format: "pem", type: "pkcs8" });
const publicB64 = (seed) => createPublicKey(pemOf(seed)).export({ format: "der", type: "spki" }).subarray(12).toString("base64");

let dir;
beforeEach(() => {
    dir = mkdtempSync(join(tmpdir(), "release-manifest-"));
    writeFileSync(join(dir, "AgentMux-0.60.1-x64-setup.exe"), "setup");
    writeFileSync(join(dir, "agentmux-0.60.1-x64-portable.7z"), "portable");
});
afterEach(() => rmSync(dir, { recursive: true, force: true }));

const release = (seed = 7) => {
    const files = listFiles(dir);
    const { manifest, envelope } = buildRelease({ files, tag: TAG, createdAt: "2026-10-25T12:00:00Z" });
    return { files, manifest, envelope, signatures: signWithKey(envelope, pemOf(seed), "k1") };
};
const verify = (r, over = {}) =>
    verifyRelease({ envelope: r.envelope, signatures: r.signatures, manifest: r.manifest, tag: TAG, publicKeyB64: publicB64(7), id: "k1", ...over });

describe("a release made and checked", () => {
    it("is made from the files, and verifies with the key that signed it and with the real files", () => {
        const r = release();
        const m = verify(r, { files: r.files });
        expect(m.files.map((f) => f.name)).toEqual(["AgentMux-0.60.1-x64-setup.exe", "agentmux-0.60.1-x64-portable.7z"].sort());
        expect(m.version).toBe("0.60.1");
        expect(JSON.parse(r.envelope).manifest_sha256).toBe(sha256Hex(r.manifest));
    });

    it("matches the fixture the Rust side checks, byte for byte", () => {
        // The same release as `a_release_made_by_the_script_verifies` (sizes and hashes spelled out).
        const files = [
            { name: "AgentMux-0.60.1-x64-setup.exe", size: 100, sha256: sha256Hex("setup") },
            { name: "agentmux-0.60.1-x64-portable.7z", size: 50, sha256: sha256Hex("portable") },
        ];
        const { manifest, envelope } = buildRelease({ files, tag: TAG, createdAt: "2026-10-25T12:00:00Z" });
        expect(publicB64(7)).toBe("6kpsY+KcUgq+9VB7Ey7F+ZVHdq6+vnuSQh7qaRRG0iw=");
        expect(JSON.parse(envelope).manifest_sha256).toBe("8a83f0bb7ce99e7537b872b55d6e688da6713ebd078b0dd88c740b1a3c5809eb");
        expect(signWithKey(envelope, pemOf(7), "k1")).toBe(
            "k1 oGZeVKdahQJP1mqd964PNT7Fie6IbSoUMkDXrmi1QbUVn947HuHtRZNRJt36L22rcAkLPGRWolco8DB2AZXZCw==\n",
        );
        expect(manifest.endsWith("}\n")).toBe(true);
    });

    it("names its files by version", () => {
        expect(versionOfTag(TAG)).toBe("0.60.1");
        expect(manifestName(TAG)).toBe("AgentMux-0.60.1-manifest.json");
        expect(envelopeName(TAG)).toBe("AgentMux-0.60.1-envelope.json");
        expect(signatureName(TAG)).toBe("AgentMux-0.60.1-envelope.json.sig");
        expect(() => versionOfTag("latest")).toThrow();
        expect(() => versionOfTag("v0.60")).toThrow();
    });

    it("keeps the signed envelope small however many files there are", () => {
        const files = Array.from({ length: 500 }, (_, i) => ({ name: `file-${i}.bin`, size: i + 1, sha256: sha256Hex(String(i)) }));
        const { manifest, envelope } = buildRelease({ files, tag: TAG });
        expect(Buffer.byteLength(manifest)).toBeGreaterThan(8192);
        expect(Buffer.byteLength(envelope)).toBeLessThan(512);
    });
});

describe("the self-check of a new signing key", () => {
    it("accepts a plain Ed25519 signature and refuses another message, key or garbage", () => {
        const message = Buffer.from("agentmux-release-signing-check");
        const sig = signWithKey(message, pemOf(7), "k1").split(" ")[1].trim();
        expect(() => selfCheck({ message, signatureB64: sig, publicKeyB64: publicB64(7) })).not.toThrow();
        expect(() => selfCheck({ message: Buffer.from("other"), signatureB64: sig, publicKeyB64: publicB64(7) })).toThrow(/plain Ed25519/);
        expect(() => selfCheck({ message, signatureB64: sig, publicKeyB64: publicB64(8) })).toThrow(/plain Ed25519/);
        expect(() => selfCheck({ message, signatureB64: "AAAA", publicKeyB64: publicB64(7) })).toThrow(/plain Ed25519/);
        expect(() => selfCheck({ message, signatureB64: sig, publicKeyB64: "AAAA" })).toThrow(/32 raw bytes/);
    });
});

describe("what must fail", () => {
    it("a signature by another key, a tampered envelope, or a missing line", () => {
        const r = release();
        expect(() => verify(release(8))).toThrow(/signature doesn't match/);
        expect(() => verify(r, { envelope: r.envelope.replace("2026-10-25", "2026-10-26") })).toThrow(/signature doesn't match/);
        expect(() => verify(r, { signatures: "" })).toThrow(/no signature line/);
        expect(() => verify(r, { id: "k2" })).toThrow(/no signature line/);
        expect(() => verify(r, { signatures: "k1 AAAA\n" })).toThrow(/signature doesn't match/);
    });

    it("an envelope replayed under another tag, or a manifest that isn't the signed one", () => {
        const r = release();
        expect(() => verify(r, { tag: "v0.60.2" })).toThrow(/is for v0.60.1, not v0.60.2/);
        expect(() => verify(r, { manifest: r.manifest.replace('"size": 5', '"size": 6') })).toThrow(/isn't the one the signature covers/);
    });

    it("a file that isn't the one in the manifest, or one it doesn't list", () => {
        const r = release();
        const swapped = r.files.map((f) => (f.name.endsWith(".exe") ? { ...f, sha256: sha256Hex("evil") } : f));
        expect(() => verify(r, { files: swapped })).toThrow(/isn't the file the manifest describes/);
        expect(() => verify(r, { files: [...r.files, { name: "extra.bin", size: 1, sha256: sha256Hex("x") }] })).toThrow(/isn't in the manifest/);
        expect(() => verify(r, { files: r.files.slice(1) })).toThrow(/is in the manifest but isn't among the files/);
    });

    it("a manifest that breaks the format, even when it is the signed one", () => {
        const check = (mutate) => {
            const m = JSON.parse(release().manifest);
            mutate(m);
            const manifest = JSON.stringify(m);
            const envelope = JSON.stringify({ format: 1, tag: TAG, manifest_sha256: sha256Hex(manifest), created_at: "x" });
            return verifyRelease({ envelope, signatures: signWithKey(envelope, pemOf(7), "k1"), manifest, tag: TAG, publicKeyB64: publicB64(7), id: "k1" });
        };
        expect(() => check(() => {})).not.toThrow();
        expect(() => check((m) => (m.files[0].name = "../evil.exe"))).toThrow(/plain file name/);
        expect(() => check((m) => (m.files[0].name = ".hidden"))).toThrow(/plain file name/);
        expect(() => check((m) => (m.files[0].name = "has space.exe"))).toThrow(/plain file name/);
        expect(() => check((m) => (m.files[0].sha256 = "ABC"))).toThrow(/SHA-256/);
        expect(() => check((m) => (m.files[0].size = 0))).toThrow(/size/);
        expect(() => check((m) => (m.files[1].name = m.files[0].name))).toThrow(/listed twice/);
        expect(() => check((m) => (m.files = []))).toThrow(/no files/);
        expect(() => check((m) => (m.format = 2))).toThrow(/format 2/);
        expect(() => check((m) => (m.version = "0.60.9"))).toThrow(/another release/);
    });

    it("an empty folder, an empty file, or an odd file name", () => {
        const empty = mkdtempSync(join(tmpdir(), "release-manifest-empty-"));
        try {
            expect(() => listFiles(empty)).toThrow(/no files/);
            writeFileSync(join(empty, "zero.bin"), "");
            expect(() => listFiles(empty)).toThrow(/is empty/);
            rmSync(join(empty, "zero.bin"));
            writeFileSync(join(empty, "has space.bin"), "x");
            expect(() => listFiles(empty)).toThrow(/plain release file name/);
        } finally {
            rmSync(empty, { recursive: true, force: true });
        }
    });
});

describe("the container images a release names", () => {
    const DIGEST = `sha256:${"ab".repeat(32)}`;
    const IMAGE = { name: "agent-base", ref: "ghcr.io/agentmuxai/agent-base", digest: DIGEST };
    const withImages = (images) => {
        const files = listFiles(dir);
        const { manifest, envelope } = buildRelease({ files, tag: TAG, createdAt: "2026-10-25T12:00:00Z", images });
        return { files, manifest, envelope, signatures: signWithKey(envelope, pemOf(7), "k1") };
    };

    it("are written after the files, signed with the manifest, and verify", () => {
        const r = withImages([IMAGE]);
        expect(verify(r, { files: r.files }).images).toEqual([IMAGE]);
        expect(Object.keys(JSON.parse(r.manifest))).toEqual(["format", "version", "tag", "files", "images"]);
        // The envelope covers the manifest, so changing the digest breaks the signature.
        const swapped = r.manifest.replace(DIGEST, `sha256:${"cd".repeat(32)}`);
        expect(() => verify(r, { manifest: swapped })).toThrow(/isn't the one the signature covers/);
    });

    it("leave a release without images byte for byte as before", () => {
        const plain = release();
        expect(withImages([]).manifest).toBe(plain.manifest);
        expect(withImages(undefined).manifest).toBe(plain.manifest);
        expect(JSON.parse(plain.manifest).images).toBeUndefined();
    });

    it("are read from --image name=path@digest", () => {
        expect(parseImageArg(`agent-base=ghcr.io/agentmuxai/agent-base@${DIGEST}`)).toEqual(IMAGE);
        for (const bad of ["", "agent-base", "ghcr.io/x/y@sha256:ab", `=ghcr.io/x/y@${DIGEST}`, `a=ghcr.io/x/y:latest`]) {
            expect(() => checkImages([parseImageArg(bad)]), bad).toThrow();
        }
    });

    it("must be well formed: name, registry path, digest, no repeats, at most 16", () => {
        const bad = (over) => () => checkImages([{ ...IMAGE, ...over }]);
        expect(() => checkImages([IMAGE, { ...IMAGE, name: "other", ref: "registry.example.com:5000/team/app" }])).not.toThrow();
        for (const name of ["", "Agent-Base", "-x", "has space", "a".repeat(65), undefined]) expect(bad({ name }), String(name)).toThrow(/plain image name/);
        for (const ref of [
            "agent-base",
            "https://ghcr.io/agentmuxai/agent-base",
            "ghcr.io/agentmuxai/agent-base:latest",
            `ghcr.io/agentmuxai/agent-base@${DIGEST}`,
            "ghcr.io//agent-base",
            "ghcr.io/AgentMux/agent-base",
            "ghcr.io/agentmuxai/agent base",
            `ghcr.io/${"a".repeat(260)}`,
            undefined,
        ]) {
            expect(bad({ ref }), String(ref)).toThrow(/registry|ref/);
        }
        for (const digest of ["", "sha256:AB", `sha256:${"ab".repeat(31)}`, `sha512:${"ab".repeat(32)}`, "latest", undefined]) {
            expect(bad({ digest }), String(digest)).toThrow(/digest/);
        }
        expect(() => checkImages([IMAGE, IMAGE])).toThrow(/listed twice/);
        const many = Array.from({ length: 17 }, (_, i) => ({ ...IMAGE, name: `img-${i}` }));
        expect(() => checkImages(many)).toThrow(/more than 16/);
        expect(() => checkImages(many.slice(0, 16))).not.toThrow();
        expect(() => checkImages("agent-base")).toThrow(/isn't a list/);
    });

    it("are refused by verify when a signed manifest carries a bad entry", () => {
        const make = (images) => {
            const m = JSON.parse(release().manifest);
            m.images = images;
            const manifest = JSON.stringify(m);
            const envelope = JSON.stringify({ format: 1, tag: TAG, manifest_sha256: sha256Hex(manifest), created_at: "x" });
            return () => verifyRelease({ envelope, signatures: signWithKey(envelope, pemOf(7), "k1"), manifest, tag: TAG, publicKeyB64: publicB64(7), id: "k1" });
        };
        expect(make([IMAGE])).not.toThrow();
        expect(make([])).not.toThrow();
        expect(make([{ ...IMAGE, digest: "sha256:zz" }])).toThrow(/digest/);
        expect(make([IMAGE, IMAGE])).toThrow(/listed twice/);
        expect(make({ name: "agent-base" })).toThrow(/isn't a list/);
    });
});

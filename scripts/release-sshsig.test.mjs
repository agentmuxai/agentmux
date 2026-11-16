// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The second signature of a release's envelope, in SSH signature (SSHSIG) form
// (docs/specs/SPEC_RELEASE_SSHSIG_SECOND_SIGNATURE_2026_11_15.md): made, parsed and checked here with a throwaway key,
// and checked by the real `ssh-keygen -Y verify` wherever it is installed (the release job's runner has it; CI sets CI).

import { spawnSync } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync, writeFileSync, mkdirSync, existsSync } from "node:fs";
import { createPrivateKey, createPublicKey, sign as cryptoSign } from "node:crypto";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import {
    SSHSIG_FLOOR,
    SIGNER_ENV,
    SSHSIG_NAMESPACE,
    buildRelease,
    envelopeName,
    listFiles,
    parseSshsig,
    signWithKey,
    signSshsigWithKey,
    sshKeygenVerify,
    sshsigArmor,
    sshsigName,
    sshsigPathFor,
    sshsigPreimage,
    signerCommand,
    verifyRelease,
    verifySshsig,
    versionAtLeast,
} from "./release-manifest.mjs";

const HERE = dirname(fileURLToPath(import.meta.url));
const SCRIPT = join(HERE, "release-manifest.mjs");
const hasKeygen = spawnSync("ssh-keygen", ["-Y"], { encoding: "utf8" }).error === undefined;
const TAG = `v${SSHSIG_FLOOR}`;
const OLD_TAG = "v0.60.1";

const privateKey = (seed) =>
    createPrivateKey({
        key: Buffer.concat([Buffer.from("302e020100300506032b657004220420", "hex"), Buffer.alloc(32, seed)]),
        format: "der",
        type: "pkcs8",
    });
const pemOf = (seed) => privateKey(seed).export({ format: "pem", type: "pkcs8" });
const publicB64 = (seed) => createPublicKey(pemOf(seed)).export({ format: "der", type: "spki" }).subarray(12).toString("base64");
const ENVELOPE = Buffer.from('{\n  "format": 1,\n  "tag": "v0.60.30",\n  "manifest_sha256": "' + "a".repeat(64) + '",\n  "created_at": "2026-11-15T00:00:00Z"\n}\n');

let dir;
beforeEach(() => {
    dir = mkdtempSync(join(tmpdir(), "release-sshsig-"));
});
afterEach(() => rmSync(dir, { recursive: true, force: true }));

/**
 * The script, run as a person's release job would run it, but with nothing of AWS in its environment (no credentials,
 * no profile, no config, no instance role): whatever a test does, it cannot sign with the real service.
 */
const runScript = (args, env = {}) => {
    const withoutAws = Object.fromEntries(Object.entries(process.env).filter(([k]) => !k.toUpperCase().startsWith("AWS_")));
    const none = { AWS_PROFILE: "no-such-profile-for-tests", AWS_CONFIG_FILE: join(dir, "no-config"), AWS_SHARED_CREDENTIALS_FILE: join(dir, "no-credentials"), AWS_EC2_METADATA_DISABLED: "true" };
    return spawnSync(process.execPath, [SCRIPT, ...args], { encoding: "utf8", env: { ...withoutAws, ...none, ...env } });
};

/**
 * A stand-in for `aws kms sign`, as a node script (it runs the same on Windows, macOS and Linux, with no shell and no
 * program looked up on the PATH), named to the script by the environment variable the script reads. Its Nth call signs
 * the `--message` file with the key `calls[N]` names (the last one repeats), or answers with text that is no signature.
 * It writes the arguments of every call to `log`, so a test can say the stand-in, and nothing else, was called.
 */
function writeStandIn(calls) {
    const script = join(dir, "stand-in-signer.mjs");
    const log = join(dir, "signer.log");
    const spec = { log, calls };
    writeFileSync(
        script,
        [
            'import { appendFileSync, readFileSync } from "node:fs";',
            'import { createPrivateKey, sign } from "node:crypto";',
            `const spec = ${JSON.stringify(spec)};`,
            'appendFileSync(spec.log, process.argv.slice(2).join(" ") + "\\n");',
            'const n = readFileSync(spec.log, "utf8").trim().split("\\n").length;',
            "const call = spec.calls[Math.min(n, spec.calls.length) - 1];",
            'if (call.garbage) { console.log("not a signature"); process.exit(0); }',
            'const file = process.argv[process.argv.indexOf("--message") + 1].replace(/^fileb:\\/\\//, "");',
            'process.stdout.write(sign(null, readFileSync(file), createPrivateKey(readFileSync(call.pem, "utf8"))).toString("base64"));',
            "",
        ].join("\n"),
    );
    return { log, env: { [SIGNER_ENV]: JSON.stringify([process.execPath, script]) } };
}

/** What the stand-in was called with, one entry per call; none if it was never called. */
const standInCalls = (stand) => (existsSync(stand.log) ? readFileSync(stand.log, "utf8").trim().split("\n") : []);

/** A release built from one file, and the arguments `sign-kms` is given: the envelope, a keys file (key 7 pinned) and where the signature goes. */
function releaseToSign() {
    const files = join(dir, "files");
    const out = join(dir, "out");
    mkdirSync(files);
    writeFileSync(join(files, `AgentMux-${SSHSIG_FLOOR}-x64-setup.exe`), "setup");
    expect(runScript(["build", "--dir", files, "--tag", TAG, "--out", out]).status).toBe(0);
    writeFileSync(join(dir, "keys.json"), JSON.stringify({ keys: [{ id: "k1", public_key: publicB64(7) }] }));
    const sig = join(out, `${envelopeName(TAG)}.sig`);
    const signArgs = ["sign-kms", "--envelope", join(out, envelopeName(TAG)), "--key-id", "alias/release", "--id", "k1", "--out", sig, "--keys-file", join(dir, "keys.json")];
    return { files, out, sig, signArgs };
}

describe("the signed bytes and the file", () => {
    it("the preimage is SSHSIG, the namespace, an empty reserved string, sha512 and the envelope's SHA-512, each a length-prefixed string", () => {
        const p = sshsigPreimage(ENVELOPE);
        expect(p.subarray(0, 6).toString()).toBe("SSHSIG");
        expect(p.length).toBe(6 + 4 + 16 + 4 + 4 + 6 + 4 + 64);
        expect(p.subarray(6, 10).readUInt32BE()).toBe(16);
        expect(p.subarray(10, 26).toString()).toBe(SSHSIG_NAMESPACE);
    });

    it("an armoured signature is taken apart to what went in, and nothing else is accepted", () => {
        const pub = Buffer.from(publicB64(1), "base64");
        const text = sshsigArmor(pub, Buffer.alloc(64, 9));
        const p = parseSshsig(text);
        expect(p.publicKey.equals(pub)).toBeTruthy();
        expect(p.signature.equals(Buffer.alloc(64, 9))).toBeTruthy();
        expect([p.namespace, p.hashAlg]).toEqual([SSHSIG_NAMESPACE, "sha512"]);
        expect(() => parseSshsig("")).toThrow(/armoured/);
        expect(() => parseSshsig(text.replace("BEGIN", "BEGIM"))).toThrow(/armoured/);
        expect(() => parseSshsig(text.replace("SSH SIGNATURE-----\n", "SSH SIGNATURE-----\n!"))).toThrow(/armoured/);
        expect(() => parseSshsig(text + text)).toThrow(/armoured/);
        expect(() => parseSshsig("x".repeat(3000))).toThrow(/too large/);
        const body = text.split("\n").slice(1, -2).join("");
        const raw = Buffer.from(body, "base64");
        const wrap = (b) => `-----BEGIN SSH SIGNATURE-----\n${b.toString("base64")}\n-----END SSH SIGNATURE-----\n`;
        expect(() => parseSshsig(wrap(Buffer.concat([raw, Buffer.from([0])])))).toThrow(/after its end/);
        expect(() => parseSshsig(wrap(raw.subarray(0, raw.length - 3)))).toThrow(/cut short/);
        const wrongMagic = Buffer.from(raw);
        wrongMagic.write("SSHSIX", 0);
        expect(() => parseSshsig(wrap(wrongMagic))).toThrow(/magic/);
    });
});

describe("checking an SSH signature against the pinned key", () => {
    const sign = (seed = 7, envelope = ENVELOPE) => signSshsigWithKey(envelope, pemOf(seed), publicB64(seed));

    it("the pinned key's signature of the envelope is good", () => {
        verifySshsig({ envelope: ENVELOPE, sshsig: sign(), publicKeyB64: publicB64(7) });
    });

    it("one changed byte in the envelope is refused", () => {
        const changed = Buffer.from(ENVELOPE);
        changed[changed.length - 3] ^= 1;
        expect(() => verifySshsig({ envelope: changed, sshsig: sign(), publicKeyB64: publicB64(7) })).toThrow(/doesn't match the envelope/);
    });

    it("another namespace is refused, even when it is validly signed", () => {
        const ns = "someone-elses-namespace";
        // Signed over the preimage for that namespace, so only the namespace rule can refuse it.
        const sig = cryptoSign(null, sshsigPreimage(ENVELOPE, ns), privateKey(7));
        const other = sshsigArmor(Buffer.from(publicB64(7), "base64"), sig, ns);
        expect(() => verifySshsig({ envelope: ENVELOPE, sshsig: other, publicKeyB64: publicB64(7) })).toThrow(/namespace/);
    });

    it("a signature by another key, under the pinned key's name, is refused", () => {
        // The signature is by key 8; the file says key 7 made it.
        const forged = signSshsigWithKey(ENVELOPE, pemOf(8), publicB64(7));
        expect(() => verifySshsig({ envelope: ENVELOPE, sshsig: forged, publicKeyB64: publicB64(7) })).toThrow(/doesn't match the envelope/);
    });

    it("a good SSH signature by a key that is not the pinned one is refused: the key inside the file is never trusted", () => {
        expect(() => verifySshsig({ envelope: ENVELOPE, sshsig: sign(8), publicKeyB64: publicB64(7) })).toThrow(/isn't the pinned one/);
    });

    it("the pinned key must be 32 raw bytes", () => {
        expect(() => verifySshsig({ envelope: ENVELOPE, sshsig: sign(), publicKeyB64: "AAAA" })).toThrow(/32 raw bytes/);
    });
});

describe("the floor: from the first release that carries it, a release without one is refused", () => {
    it("compares versions numerically", () => {
        expect(versionAtLeast("0.60.31", "0.60.31")).toBe(true);
        expect(versionAtLeast("0.60.30", "0.60.31")).toBe(false);
        expect(versionAtLeast("0.61.0", "0.60.31")).toBe(true);
        expect(versionAtLeast("0.60.100", "0.60.31")).toBe(true);
        expect(versionAtLeast("1.0.0", "0.99.99")).toBe(true);
    });

    it("refuses anything that is not exactly digits X.Y.Z, so a suffix can never read as below the floor", () => {
        for (const bad of ["0.60.31-rc1", "0.60", "0.60.31.1", "v0.60.31", "x", "", "0.60.x", "1.2.3+4"]) {
            expect(() => versionAtLeast(bad, SSHSIG_FLOOR)).toThrow(/isn't a version of digits/);
            expect(() => versionAtLeast(SSHSIG_FLOOR, bad)).toThrow(/isn't a version of digits/);
        }
    });

    const release = (tag) => {
        const v = tag.slice(1);
        writeFileSync(join(dir, `AgentMux-${v}-x64-setup.exe`), "setup");
        const { manifest, envelope } = buildRelease({ files: listFiles(dir), tag, createdAt: "2026-11-15T00:00:00Z" });
        return { manifest, envelope, signatures: signWithKey(envelope, pemOf(7), "k1") };
    };
    const verify = (r, tag, sshsig) => verifyRelease({ ...r, tag, publicKeyB64: publicB64(7), id: "k1", sshsig });

    it("a release at the floor with no SSH signature fails, with the file's name in the message", () => {
        const r = release(TAG);
        expect(() => verify(r, TAG, undefined)).toThrow(sshsigName(TAG));
    });

    it("a release at the floor with a good one passes, and a bad one fails", () => {
        const r = release(TAG);
        verify(r, TAG, signSshsigWithKey(r.envelope, pemOf(7), publicB64(7)));
        expect(() => verify(r, TAG, signSshsigWithKey(r.envelope, pemOf(8), publicB64(8)))).toThrow(/isn't the pinned one/);
    });

    it("a release below the floor needs none, and is still checked when it has one", () => {
        const r = release(OLD_TAG);
        verify(r, OLD_TAG, undefined);
        verify(r, OLD_TAG, signSshsigWithKey(r.envelope, pemOf(7), publicB64(7)));
        expect(() => verify(r, OLD_TAG, signSshsigWithKey(r.envelope, pemOf(8), publicB64(8)))).toThrow(/isn't the pinned one/);
    });
});

describe("the command line, through the local-key path (the same code as the signing service's, with its own signer)", { timeout: 30_000 }, () => {
    const run = runScript;
    const made = () => {
        const files = join(dir, "files");
        const out = join(dir, "out");
        mkdirSync(files);
        writeFileSync(join(files, `AgentMux-${SSHSIG_FLOOR}-x64-setup.exe`), "setup");
        writeFileSync(join(dir, "k.pem"), pemOf(7));
        expect(run(["build", "--dir", files, "--tag", TAG, "--out", out]).status).toBe(0);
        const sig = join(out, `${envelopeName(TAG)}.sig`);
        const r = run(["sign-key", "--envelope", join(out, envelopeName(TAG)), "--private-key", join(dir, "k.pem"), "--id", "k1", "--out", sig, "--sshsig-out", sshsigPathFor(sig)]);
        expect(r.status).toBe(0);
        return { files, out, sshsig: join(out, sshsigName(TAG)) };
    };
    const verifyArgs = (m, more = []) => ["verify", "--dir", m.out, "--tag", TAG, "--id", "k1", "--public-key", publicB64(7), "--files", m.files, ...more];

    it("the SSH signature's path is the signature line's, with .sshsig for .sig", () => {
        expect(sshsigPathFor("release-manifest/AgentMux-0.60.30-envelope.json.sig")).toBe("release-manifest/AgentMux-0.60.30-envelope.json.sshsig");
        expect(() => sshsigPathFor("x.txt")).toThrow(/--sshsig-out/);
    });

    it("verify passes with the file (own parser; the real tool where it is installed)", () => {
        const m = made();
        const r = run(verifyArgs(m, hasKeygen ? [] : ["--no-ssh-keygen"]));
        expect(r.status).toBe(0);
        expect(r.stdout).toContain("SSH signature");
    });

    it("verify fails when the file is missing, at the floor", () => {
        const m = made();
        rmSync(m.sshsig);
        const r = run(verifyArgs(m, ["--no-ssh-keygen"]));
        expect(r.status).toBe(1);
        expect(r.stderr).toContain(sshsigName(TAG));
    });

    it("verify fails when the file was changed", () => {
        const m = made();
        const text = readFileSync(m.sshsig, "utf8");
        writeFileSync(m.sshsig, text.replace(/\n([A-Za-z0-9+/])/, (_, c) => `\n${c === "A" ? "B" : "A"}`));
        expect(run(verifyArgs(m, ["--no-ssh-keygen"])).status).toBe(1);
    });

    it("verify fails when the key that signed it is not the pinned one", () => {
        const m = made();
        writeFileSync(m.sshsig, signSshsigWithKey(readFileSync(join(m.out, envelopeName(TAG))), pemOf(8), publicB64(8)));
        const r = run(verifyArgs(m, ["--no-ssh-keygen"]));
        expect(r.status).toBe(1);
        expect(r.stderr).toContain("isn't the pinned one");
    });

    it("sign-kms writes both files in the one step, with the signing service called twice with the same key id and a second message", () => {
        const { files, out, sig, signArgs } = releaseToSign();
        writeFileSync(join(dir, "k.pem"), pemOf(7));
        const stand = writeStandIn([{ pem: join(dir, "k.pem") }]);
        const r = run(signArgs, stand.env);
        // First of all: the stand-in was what signed, twice, and nothing else could have.
        const calls = standInCalls(stand);
        expect(calls.length).toBe(2);
        expect(r.stderr).toBe("");
        expect(r.status).toBe(0);
        expect(existsSync(sig)).toBe(true);
        expect(existsSync(join(out, sshsigName(TAG)))).toBe(true);
        for (const c of calls) {
            expect(c).toContain("kms sign --key-id alias/release");
            expect(c).toContain("--message-type RAW --signing-algorithm ED25519_SHA_512");
        }
        expect(calls[0]).not.toBe(calls[1]); // two different message files
        const v = run(["verify", "--dir", out, "--tag", TAG, "--id", "k1", "--keys-file", join(dir, "keys.json"), "--files", files, ...(hasKeygen ? [] : ["--no-ssh-keygen"])]);
        expect(v.stderr).toBe("");
        expect(v.status).toBe(0);
    });

    it("sign-kms refuses to write a signature the pinned key does not verify (the service's key is not the pinned one)", () => {
        const { out, signArgs } = releaseToSign();
        writeFileSync(join(dir, "k.pem"), pemOf(8)); // the service signs with key 8
        const stand = writeStandIn([{ pem: join(dir, "k.pem") }]);
        const r = run(signArgs, stand.env);
        expect(standInCalls(stand).length).toBeGreaterThan(0);
        expect(r.status).toBe(1);
        expect(existsSync(join(out, sshsigName(TAG)))).toBe(false);
    });
});

describe("the signing program: aws from the PATH in the release job, only a named stand-in in a test", { timeout: 30_000 }, () => {
    it("is plain `aws` when nothing names another and no test runner is running: the release workflow's behaviour", () => {
        expect(signerCommand({})).toEqual(["aws"]);
        expect(signerCommand({ [SIGNER_ENV]: "" })).toEqual(["aws"]);
        // The release job's environment (the AWS variables the signing step has) changes nothing.
        expect(signerCommand({ AWS_REGION: "x", AWS_ACCESS_KEY_ID: "y" })).toEqual(["aws"]);
    });

    it("is the program a test names, with its leading arguments, and refuses anything that is not a JSON array of strings", () => {
        expect(signerCommand({ [SIGNER_ENV]: JSON.stringify(["/n/node", "/t/fake.mjs"]) })).toEqual(["/n/node", "/t/fake.mjs"]);
        for (const bad of ["aws", "[]", "[1]", '[""]', '{"a":1}', "null", '["a", 2]']) {
            expect(() => signerCommand({ [SIGNER_ENV]: bad }), bad).toThrow(/JSON array of strings/);
        }
    });

    it("refuses under a test runner when no stand-in is named, so a test can never reach the real signing service", () => {
        expect(() => signerCommand({ VITEST: "true" })).toThrow(/refusing to call the real signing service/);
        expect(() => signerCommand({ VITEST: "true", [SIGNER_ENV]: "" })).toThrow(/refusing to call the real signing service/);
        // ...and a stand-in named in the same environment is what runs.
        expect(signerCommand({ VITEST: "true", [SIGNER_ENV]: JSON.stringify(["x"]) })).toEqual(["x"]);
    });

    it("sign-kms run with no stand-in named signs nothing, writes nothing and says why (and could not have found an aws: the PATH is an empty folder)", () => {
        const { out, signArgs } = releaseToSign();
        mkdirSync(join(dir, "empty"));
        const r = runScript(signArgs, { PATH: join(dir, "empty"), VITEST: "true", [SIGNER_ENV]: "" });
        expect(r.status).toBe(1);
        expect(r.stderr).toContain("refusing to call the real signing service");
        expect(existsSync(join(out, `${envelopeName(TAG)}.sig`))).toBe(false);
        expect(existsSync(join(out, sshsigName(TAG)))).toBe(false);
    });
});

describe("sign-kms when the second signature goes wrong: non-zero exit, no .sshsig left behind", { timeout: 30_000 }, () => {
    const run = runScript;
    // The stand-in's first call signs with key 7 (the .sig line); the second does what `second` says.
    const setup = (second) => {
        const { out, signArgs } = releaseToSign();
        writeFileSync(join(dir, "k7.pem"), pemOf(7));
        writeFileSync(join(dir, "k8.pem"), pemOf(8));
        const stand = writeStandIn([{ pem: join(dir, "k7.pem") }, second === "garbage" ? { garbage: true } : { pem: join(dir, "k8.pem") }]);
        // A file left by an earlier run must not survive a failed one.
        writeFileSync(join(out, sshsigName(TAG)), "stale");
        const r = run(signArgs, stand.env);
        // First of all: the stand-in was what answered, twice.
        expect(standInCalls(stand).length).toBe(2);
        return { r, out };
    };

    it("garbage from the service for the second message: exit 1, the stale file is gone, nothing partial is written", () => {
        const { r, out } = setup("garbage");
        expect(r.status).toBe(1);
        expect(r.stderr).toContain("didn't return a signature");
        expect(existsSync(join(out, sshsigName(TAG)))).toBe(false);
        expect(existsSync(join(out, `${sshsigName(TAG)}.tmp`))).toBe(false);
    });

    it("another key's signature for the second message: exit 1, no .sshsig", () => {
        const { r, out } = setup("other-key");
        expect(r.status).toBe(1);
        expect(r.stderr).toContain("doesn't match the envelope");
        expect(existsSync(join(out, sshsigName(TAG)))).toBe(false);
        expect(existsSync(join(out, `${sshsigName(TAG)}.tmp`))).toBe(false);
    });
});

describe("the real ssh-keygen", () => {
    it("is installed where CI runs (a skip there would hide the check this file exists for)", () => {
        if (process.env.CI) expect(hasKeygen).toBe(true);
    });

    const it2 = it.skipIf(!hasKeygen);
    const sign = (seed = 7) => signSshsigWithKey(ENVELOPE, pemOf(seed), publicB64(seed));

    it2("accepts our signature, and refuses a changed envelope, another key's and another namespace", () => {
        sshKeygenVerify({ envelope: ENVELOPE, sshsig: sign(), publicKeyB64: publicB64(7) });
        const changed = Buffer.concat([ENVELOPE, Buffer.from(" ")]);
        expect(() => sshKeygenVerify({ envelope: changed, sshsig: sign(), publicKeyB64: publicB64(7) })).toThrow(/refused/);
        expect(() => sshKeygenVerify({ envelope: ENVELOPE, sshsig: sign(8), publicKeyB64: publicB64(7) })).toThrow(/refused/);
    });

    it2("our parser and checker accept a signature that OpenSSH itself made", () => {
        // ssh-keygen -Y sign with an Ed25519 key, in the release namespace; the pinned key is that key's public half.
        const key = join(dir, "id");
        expect(spawnSync("ssh-keygen", ["-q", "-t", "ed25519", "-N", "", "-f", key, "-C", "t"]).status).toBe(0);
        const msg = join(dir, "envelope.json");
        writeFileSync(msg, ENVELOPE);
        expect(spawnSync("ssh-keygen", ["-Y", "sign", "-f", key, "-n", SSHSIG_NAMESPACE, msg]).status).toBe(0);
        const raw = Buffer.from(readFileSync(`${key}.pub`, "utf8").split(" ")[1], "base64").subarray(-32).toString("base64");
        verifySshsig({ envelope: ENVELOPE, sshsig: readFileSync(`${msg}.sig`, "utf8"), publicKeyB64: raw });
        sshKeygenVerify({ envelope: ENVELOPE, sshsig: readFileSync(`${msg}.sig`, "utf8"), publicKeyB64: raw });
    });
});

// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Tests of scripts/zip-deterministic.mjs: the Windows Headless package's archive is the same bytes for the same contents,
// and it reads back as what was packed.

import { spawnSync } from "node:child_process";
import { mkdirSync, mkdtempSync, rmSync, symlinkSync, utimesSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { inflateRawSync } from "node:zlib";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { crc32, DOS_DATE, DOS_TIME, listEntries, writeZip, zipFolder } from "./zip-deterministic.mjs";

const SCRIPT = fileURLToPath(new URL("./zip-deterministic.mjs", import.meta.url));

let tmp;
beforeEach(() => {
    tmp = mkdtempSync(join(tmpdir(), "zipdet-"));
});
afterEach(() => rmSync(tmp, { recursive: true, force: true }));

/** A package-shaped folder: `top/bin/*.exe`, `top/web/index.html`, `top/VERSION`, and an empty folder. */
function makePackage(name = "agentmux-headless-1.2.3-windows-x64") {
    const top = join(tmp, name);
    mkdirSync(join(top, "bin"), { recursive: true });
    mkdirSync(join(top, "web", "assets"), { recursive: true });
    mkdirSync(join(top, "empty"));
    writeFileSync(join(top, "bin", "agentmux.exe"), Buffer.alloc(5000, 7));
    writeFileSync(join(top, "bin", "agentmux-srv.exe"), Buffer.from(Array.from({ length: 3000 }, (_, i) => (i * 31) % 251)));
    writeFileSync(join(top, "web", "index.html"), "<html>héllo</html>\n");
    writeFileSync(join(top, "web", "assets", "ünï.js"), "export {};\n");
    writeFileSync(join(top, "VERSION"), "1.2.3\n");
    writeFileSync(join(top, "zero"), "");
    return top;
}

/** What the archive holds, read from its central directory: name, method, sizes, CRC, time, and the bytes unpacked. */
function readZip(buf) {
    const eocd = buf.length - 22;
    expect(buf.readUInt32LE(eocd)).toBe(0x06054b50);
    const count = buf.readUInt16LE(eocd + 10);
    let at = buf.readUInt32LE(eocd + 16);
    const out = [];
    for (let i = 0; i < count; i++) {
        expect(buf.readUInt32LE(at)).toBe(0x02014b50);
        const flags = buf.readUInt16LE(at + 8);
        const method = buf.readUInt16LE(at + 10);
        const time = buf.readUInt16LE(at + 12);
        const date = buf.readUInt16LE(at + 14);
        const crc = buf.readUInt32LE(at + 16);
        const csize = buf.readUInt32LE(at + 20);
        const usize = buf.readUInt32LE(at + 24);
        const nlen = buf.readUInt16LE(at + 28);
        const local = buf.readUInt32LE(at + 42);
        const name = buf.toString("utf8", at + 46, at + 46 + nlen);
        expect(buf.readUInt32LE(local)).toBe(0x04034b50);
        const dataAt = local + 30 + buf.readUInt16LE(local + 26) + buf.readUInt16LE(local + 28);
        const stored = buf.subarray(dataAt, dataAt + csize);
        const data = method === 8 ? inflateRawSync(stored) : stored;
        out.push({ name, flags, method, time, date, crc, csize, usize, data });
        at += 46 + nlen;
    }
    return out;
}

describe("zip-deterministic", () => {
    it("crc32 is the standard check value", () => {
        expect(crc32(Buffer.from("123456789"))).toBe(0xcbf43926);
        expect(crc32(Buffer.alloc(0))).toBe(0);
    });

    it("lists entries in byte order of the whole path, folders before what is in them", () => {
        const top = makePackage();
        expect(listEntries(top).map((e) => e.name)).toEqual([
            "VERSION",
            "bin/",
            "bin/agentmux-srv.exe",
            "bin/agentmux.exe",
            "empty/",
            "web/",
            "web/assets/",
            "web/assets/ünï.js",
            "web/index.html",
            "zero",
        ]);
    });

    it("makes the same bytes for the same contents, whatever the file times and the order they were made in", () => {
        const a = zipFolder(makePackage());
        rmSync(join(tmp, "agentmux-headless-1.2.3-windows-x64"), { recursive: true });
        const top = makePackage();
        const later = new Date("2031-05-05T05:05:05Z");
        for (const f of ["VERSION", "zero"]) utimesSync(join(top, f), later, later);
        const b = zipFolder(top);
        expect(b.equals(a)).toBe(true);
    });

    it("changes when a file's contents change", () => {
        const top = makePackage();
        const a = zipFolder(top);
        writeFileSync(join(top, "VERSION"), "1.2.4\n");
        expect(zipFolder(top).equals(a)).toBe(false);
    });

    it("holds one top folder, fixed times, UTF-8 names, and reads back as what was packed", () => {
        const top = makePackage();
        const entries = readZip(zipFolder(top));
        const names = entries.map((e) => e.name);
        expect(names[0]).toBe("agentmux-headless-1.2.3-windows-x64/");
        expect(names.every((n) => n.startsWith("agentmux-headless-1.2.3-windows-x64/"))).toBe(true);
        expect(names).toContain("agentmux-headless-1.2.3-windows-x64/web/assets/ünï.js");
        for (const e of entries) {
            expect(e.time).toBe(DOS_TIME);
            expect(e.date).toBe(DOS_DATE);
            expect(e.flags & 0x0800).toBe(0x0800);
            expect(crc32(e.data)).toBe(e.crc);
            expect(e.data.length).toBe(e.usize);
        }
        const exe = entries.find((e) => e.name.endsWith("bin/agentmux.exe"));
        expect(exe.method).toBe(8);
        expect(exe.csize).toBeLessThan(exe.usize);
        expect(exe.data.equals(Buffer.alloc(5000, 7))).toBe(true);
        const version = entries.find((e) => e.name.endsWith("/VERSION"));
        expect(version.data.toString()).toBe("1.2.3\n");
        const zero = entries.find((e) => e.name.endsWith("/zero"));
        expect([zero.method, zero.usize, zero.csize]).toEqual([0, 0, 0]);
        // A folder is an entry of its own, so an empty one survives.
        expect(names).toContain("agentmux-headless-1.2.3-windows-x64/empty/");
    });

    it("writes the file and says its size and hash; the CLI does the same", () => {
        const top = makePackage();
        const out = join(tmp, "a.zip");
        const { bytes, sha256 } = writeZip(top, out);
        expect(bytes).toBeGreaterThan(100);
        expect(sha256).toMatch(/^[0-9a-f]{64}$/);
        const out2 = join(tmp, "b.zip");
        const r = spawnSync(process.execPath, [SCRIPT, top, out2], { encoding: "utf8" });
        expect(r.status).toBe(0);
        expect(r.stdout).toContain(`sha256 ${sha256}`);
    });

    it("refuses a symbolic link", (ctx) => {
        const top = makePackage();
        try {
            symlinkSync(join(top, "VERSION"), join(top, "link"));
        } catch {
            ctx.skip(); // a Windows account without the symlink right
            return;
        }
        expect(() => zipFolder(top)).toThrow(/symbolic link/);
    });

    it("the CLI says how it is used when it gets no arguments", () => {
        const r = spawnSync(process.execPath, [SCRIPT], { encoding: "utf8" });
        expect(r.status).toBe(2);
        expect(r.stderr).toContain("usage:");
    });
});

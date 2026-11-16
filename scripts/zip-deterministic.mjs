#!/usr/bin/env node
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Write a folder as a .zip whose bytes depend only on its contents: entries in byte order of their names, a fixed time
// stamp, no extra fields, no owner or permission bits. The Windows Headless package is made this way, as the
// Linux and macOS tarballs are (scripts/package-server.sh: fixed order, owner and time), so the same programs always make
// the same file and its hash in the release's signed manifest can be reproduced.
//
//   node scripts/zip-deterministic.mjs <folder> <out.zip>
//
// The folder's own name is the one top folder of the archive. No dependency but Node: a Windows runner and a
// tester's machine have no `zip` program, and Compress-Archive does not keep a fixed order or time.
// Limits: no archive or file of 4 GiB or more (no zip64), no symlinks (a link is refused, not followed).

import { createHash } from "node:crypto";
import { lstatSync, readdirSync, readFileSync, writeFileSync } from "node:fs";
import { basename, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import * as zlib from "node:zlib";

/** 2026-01-01 00:00:00 as the DOS date and time a zip header holds. */
export const DOS_DATE = ((2026 - 1980) << 9) | (1 << 5) | 1;
export const DOS_TIME = 0;

const LIMIT = 0xffffffff;

let table;
/** CRC-32 of a buffer, by the table: what `crc32` falls back to on a Node that has no `zlib.crc32` (before 22.2). */
export function crc32Table(buf) {
    if (!table) {
        table = new Uint32Array(256);
        for (let n = 0; n < 256; n++) {
            let c = n;
            for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
            table[n] = c >>> 0;
        }
    }
    let c = 0xffffffff;
    for (let i = 0; i < buf.length; i++) c = table[(c ^ buf[i]) & 0xff] ^ (c >>> 8);
    return (c ^ 0xffffffff) >>> 0;
}

/** CRC-32 of a buffer: Node's own where it has one (22.2 and later), else the table form. The module is imported as a
 *  namespace: a named import of a function an older Node does not export would fail to load, and the fallback never run. */
export function crc32(buf) {
    return typeof zlib.crc32 === "function" ? zlib.crc32(buf) >>> 0 : crc32Table(buf);
}

/** Every file and folder under `dir`, as paths relative to it with `/`, in byte order of the whole path. */
export function listEntries(dir, prefix = "") {
    const out = [];
    for (const name of readdirSync(dir)) {
        const rel = prefix ? `${prefix}/${name}` : name;
        const full = join(dir, name);
        const st = lstatSync(full);
        if (st.isSymbolicLink()) throw new Error(`zip-deterministic: ${full} is a symbolic link: not packed`);
        if (st.isDirectory()) {
            out.push({ name: `${rel}/`, dir: true });
            out.push(...listEntries(full, rel));
        } else if (st.isFile()) {
            out.push({ name: rel, dir: false, full });
        } else {
            throw new Error(`zip-deterministic: ${full} is neither a file nor a folder`);
        }
    }
    return out.sort((a, b) => Buffer.compare(Buffer.from(a.name), Buffer.from(b.name)));
}

/** The archive of `root` as a Buffer: one top folder, named as `root` is. */
export function zipFolder(root) {
    const top = basename(resolve(root));
    const entries = [{ name: `${top}/`, dir: true }, ...listEntries(root).map((e) => ({ ...e, name: `${top}/${e.name}` }))];
    entries.sort((a, b) => Buffer.compare(Buffer.from(a.name), Buffer.from(b.name)));

    const parts = [];
    const central = [];
    let offset = 0;
    for (const e of entries) {
        const name = Buffer.from(e.name, "utf8");
        const raw = e.dir ? Buffer.alloc(0) : readFileSync(e.full);
        const crc = crc32(raw);
        let method = 0;
        let data = raw;
        if (raw.length > 0) {
            const packed = zlib.deflateRawSync(raw, { level: 9 });
            if (packed.length < raw.length) {
                method = 8;
                data = packed;
            }
        }
        if (raw.length > LIMIT || data.length > LIMIT || offset > LIMIT) throw new Error(`zip-deterministic: ${e.name} is too large for a zip without zip64`);
        // Bit 11: the names are UTF-8.
        const flags = 0x0800;
        const local = Buffer.alloc(30);
        local.writeUInt32LE(0x04034b50, 0);
        local.writeUInt16LE(20, 4);
        local.writeUInt16LE(flags, 6);
        local.writeUInt16LE(method, 8);
        local.writeUInt16LE(DOS_TIME, 10);
        local.writeUInt16LE(DOS_DATE, 12);
        local.writeUInt32LE(crc, 14);
        local.writeUInt32LE(data.length, 18);
        local.writeUInt32LE(raw.length, 22);
        local.writeUInt16LE(name.length, 26);
        local.writeUInt16LE(0, 28);
        parts.push(local, name, data);

        const head = Buffer.alloc(46);
        head.writeUInt32LE(0x02014b50, 0);
        head.writeUInt16LE(20, 4);
        head.writeUInt16LE(20, 6);
        head.writeUInt16LE(flags, 8);
        head.writeUInt16LE(method, 10);
        head.writeUInt16LE(DOS_TIME, 12);
        head.writeUInt16LE(DOS_DATE, 14);
        head.writeUInt32LE(crc, 16);
        head.writeUInt32LE(data.length, 20);
        head.writeUInt32LE(raw.length, 24);
        head.writeUInt16LE(name.length, 28);
        head.writeUInt32LE(e.dir ? 0x10 : 0x20, 38); // MS-DOS attributes: folder, or archive
        head.writeUInt32LE(offset, 42);
        central.push(head, name);
        offset += 30 + name.length + data.length;
    }
    const centralBytes = Buffer.concat(central);
    if (offset + centralBytes.length > LIMIT || entries.length > 0xffff) throw new Error("zip-deterministic: the archive is too large for a zip without zip64");
    const end = Buffer.alloc(22);
    end.writeUInt32LE(0x06054b50, 0);
    end.writeUInt16LE(entries.length, 8);
    end.writeUInt16LE(entries.length, 10);
    end.writeUInt32LE(centralBytes.length, 12);
    end.writeUInt32LE(offset, 16);
    return Buffer.concat([...parts, centralBytes, end]);
}

/** Write the archive of `folder` to `out`; resolves to its size and SHA-256. */
export function writeZip(folder, out) {
    const bytes = zipFolder(folder);
    writeFileSync(out, bytes);
    return { bytes: bytes.length, sha256: createHash("sha256").update(bytes).digest("hex") };
}

// Only when started as the main file (a drive letter's case or a path form must not make this silently false: the same
// path-equality as an import of this file by the tests, which skips it).
if (process.argv[1] && resolve(process.argv[1]).toLowerCase() === fileURLToPath(import.meta.url).toLowerCase()) {
    const [folder, out] = process.argv.slice(2);
    if (!folder || !out) {
        console.error("usage: node scripts/zip-deterministic.mjs <folder> <out.zip>");
        process.exit(2);
    }
    try {
        const { bytes, sha256 } = writeZip(folder, out);
        console.log(`zip-deterministic: ${out} (${bytes} bytes, sha256 ${sha256})`);
    } catch (e) {
        console.error(String(e instanceof Error ? e.message : e));
        process.exit(1);
    }
}

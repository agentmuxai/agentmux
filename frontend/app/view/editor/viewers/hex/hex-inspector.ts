// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The hex viewer's data inspector: the bytes at the cursor read as each
 * common type, in either byte order. Pure.
 * docs/specs/SPEC_EDITOR_REVAMP_UNIFIED_VIEWERS_2026_10_15.md §4.4.5.
 */

export interface Inspected {
    label: string;
    /** The value, or "" when the file ends before the type does. */
    value: string;
}

/** Bytes the inspector reads from the cursor: the longest type (ULEB128 of a u64). */
export const INSPECT_BYTES = 10;

/** IEEE 754 half precision, from its 16 bits. */
export function float16(bits: number): number {
    const sign = bits & 0x8000 ? -1 : 1;
    const exponent = (bits >> 10) & 0x1f;
    const fraction = bits & 0x3ff;
    if (exponent === 0) return sign * 2 ** -14 * (fraction / 1024);
    if (exponent === 0x1f) return fraction ? Number.NaN : sign * Number.POSITIVE_INFINITY;
    return sign * 2 ** (exponent - 15) * (1 + fraction / 1024);
}

/** An unsigned LEB128 number at the start of `bytes`; "" when it doesn't end within them. */
export function uleb128(bytes: Uint8Array): string {
    let value = BigInt(0);
    for (let i = 0; i < bytes.length && i < INSPECT_BYTES; i++) {
        value |= BigInt(bytes[i] & 0x7f) << BigInt(7 * i);
        if (!(bytes[i] & 0x80)) return value.toString();
    }
    return "";
}

/** The UTF-8 character at the start of `bytes`, as "U+00E9 é"; "" when it isn't one. */
export function utf8Char(bytes: Uint8Array): string {
    const b = bytes[0];
    if (b === undefined) return "";
    const length = b < 0x80 ? 1 : b >= 0xc2 && b < 0xe0 ? 2 : b >= 0xe0 && b < 0xf0 ? 3 : b >= 0xf0 && b < 0xf5 ? 4 : 0;
    if (!length || bytes.length < length) return "";
    let text: string;
    try {
        text = new TextDecoder("utf-8", { fatal: true }).decode(bytes.subarray(0, length));
    } catch {
        return "";
    }
    const code = text.codePointAt(0) ?? 0;
    const shown = code < 0x20 || code === 0x7f ? "" : ` ${text}`;
    return `U+${code.toString(16).toUpperCase().padStart(4, "0")}${shown}`;
}

function number(n: number): string {
    return Number.isNaN(n) ? "NaN" : Number.isFinite(n) ? String(n) : n > 0 ? "+Infinity" : "-Infinity";
}

/** The bytes at the cursor (`bytes[0]` is the cursor's), as each type. */
export function inspect(bytes: Uint8Array, littleEndian: boolean): Inspected[] {
    const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
    const read = (need: number, get: () => string): string => (bytes.length >= need ? get() : "");
    return [
        { label: "Binary", value: read(1, () => bytes[0].toString(2).padStart(8, "0")) },
        { label: "Octal", value: read(1, () => bytes[0].toString(8).padStart(3, "0")) },
        { label: "uint8", value: read(1, () => String(view.getUint8(0))) },
        { label: "int8", value: read(1, () => String(view.getInt8(0))) },
        { label: "uint16", value: read(2, () => String(view.getUint16(0, littleEndian))) },
        { label: "int16", value: read(2, () => String(view.getInt16(0, littleEndian))) },
        { label: "uint32", value: read(4, () => String(view.getUint32(0, littleEndian))) },
        { label: "int32", value: read(4, () => String(view.getInt32(0, littleEndian))) },
        { label: "uint64", value: read(8, () => view.getBigUint64(0, littleEndian).toString()) },
        { label: "int64", value: read(8, () => view.getBigInt64(0, littleEndian).toString()) },
        { label: "float16", value: read(2, () => number(float16(view.getUint16(0, littleEndian)))) },
        { label: "float32", value: read(4, () => number(view.getFloat32(0, littleEndian))) },
        { label: "float64", value: read(8, () => number(view.getFloat64(0, littleEndian))) },
        { label: "ULEB128", value: uleb128(bytes) },
        { label: "UTF-8", value: utf8Char(bytes) },
    ];
}

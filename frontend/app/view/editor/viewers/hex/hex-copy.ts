// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * What the hex viewer puts on the clipboard for a selection: hex pairs, a C
 * array, base64, or the decoded text. Pure.
 * docs/specs/SPEC_EDITOR_REVAMP_UNIFIED_VIEWERS_2026_10_15.md §4.4.5.
 */

import { charOf, hexOf } from "./hex-layout";

/** The longest selection a copy takes: its text is three to six times as long. */
export const COPY_MAX_BYTES = 1_048_576;

export type CopyAs = "hex" | "c" | "base64" | "text";

/** "DE AD BE EF" */
export function asHex(bytes: Uint8Array): string {
    return Array.from(bytes, hexOf).join(" ");
}

/** A C array initializer, twelve bytes to a line. */
export function asCArray(bytes: Uint8Array): string {
    const lines: string[] = [];
    for (let i = 0; i < bytes.length; i += 12) {
        lines.push(`    ${Array.from(bytes.subarray(i, i + 12), (b) => `0x${hexOf(b)}`).join(", ")}`);
    }
    return `unsigned char data[${bytes.length}] = {\n${lines.join(",\n")}\n};`;
}

export function asBase64(bytes: Uint8Array): string {
    let binary = "";
    // In pieces: one call with a million arguments overflows the stack.
    for (let i = 0; i < bytes.length; i += 0x8000) binary += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
    return btoa(binary);
}

/** The decoded column's text: ASCII, a dot for every other byte. */
export function asText(bytes: Uint8Array): string {
    return Array.from(bytes, charOf).join("");
}

export function copyText(bytes: Uint8Array, as: CopyAs): string {
    return as === "hex" ? asHex(bytes) : as === "c" ? asCArray(bytes) : as === "base64" ? asBase64(bytes) : asText(bytes);
}

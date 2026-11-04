// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// True when `filePath` lives under `root` — handles both POSIX and
// Windows separators, case-insensitive on Windows drives.
export function workspaceCovers(root: string | null, filePath: string): boolean {
    if (!root) return false;
    const normRoot = root.replace(/\\/g, "/").replace(/\/+$/, "");
    const normFile = filePath.replace(/\\/g, "/");
    const isWin = /^[a-zA-Z]:\//.test(normRoot);
    const r = isWin ? normRoot.toLowerCase() : normRoot;
    const f = isWin ? normFile.toLowerCase() : normFile;
    return f === r || f.startsWith(r + "/");
}

// file:// → OS path (inverse of pathToFileUri in lsp-client.ts).
// Percent-decodes each segment to undo the encoding applied on the way out.
export function decodeFileUri(uri: string): string {
    const isWin = uri.startsWith("file:///") && /^file:\/\/\/[a-zA-Z]:/.test(uri);
    const body = uri.startsWith("file:///")
        ? uri.slice(isWin ? 8 : 7) // Windows strips the leading "/" before the drive
        : uri.startsWith("file://")
          ? uri.slice(7)
          : uri;
    const decoded = body
        .split("/")
        .map((seg) => {
            try {
                return decodeURIComponent(seg);
            } catch {
                return seg;
            }
        })
        .join("/");
    return isWin ? decoded.replace(/\//g, "\\") : decoded;
}

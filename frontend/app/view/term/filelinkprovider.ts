// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import type { ILink, ILinkProvider, Terminal } from "@xterm/xterm";

// Patterns ordered by specificity — earlier matches take priority
const FILE_PATH_REGEXES: RegExp[] = [
    // Windows absolute: C:\Users\foo\bar.ts, C:/Users/foo/bar.ts
    /[A-Za-z]:[\\\/][\w.\-\\\/]+[\w.\-]/g,

    // Unix absolute: /home/user/file.ts, /usr/bin/node
    /\/[\w.\-]+(?:\/[\w.\-]+)+/g,

    // Relative with extension: ./src/app.ts, ../config/settings.json, src/components/App.tsx
    /\.{0,2}\/[\w.\-]+(?:\/[\w.\-]+)*\.[\w]+/g,

    // Home dir: ~/Documents/file.txt, ~/.config/settings.json
    /~\/[\w.\-]+(?:\/[\w.\-]+)*/g,
];

// Match trailing :line or :line:col suffix
const LINE_COL_SUFFIX = /^(.+):(\d+)(?::(\d+))?$/;

export class FilePathLinkProvider implements ILinkProvider {
    constructor(
        private readonly terminal: Terminal,
        private readonly handler: (path: string) => void
    ) {}

    provideLinks(lineNumber: number, callback: (links: ILink[] | undefined) => void): void {
        const line = this.getLineText(lineNumber);
        if (!line) {
            callback(undefined);
            return;
        }

        const links: ILink[] = [];
        for (const match of this.findPaths(line)) {
            links.push({
                range: {
                    start: { x: match.start + 1, y: lineNumber },
                    end: { x: match.end, y: lineNumber },
                },
                text: match.text,
                decorations: { pointerCursor: true, underline: true },
                activate: (_event, text) => {
                    this.handler(text);
                },
            });
        }
        callback(links.length > 0 ? links : undefined);
    }

    private getLineText(lineNumber: number): string | undefined {
        const buffer = this.terminal.buffer.active;
        const line = buffer.getLine(lineNumber - 1);
        return line?.translateToString(true);
    }

    private findPaths(text: string): Array<{ start: number; end: number; text: string }> {
        const results: Array<{ start: number; end: number; text: string }> = [];

        for (const regex of FILE_PATH_REGEXES) {
            regex.lastIndex = 0;
            let match: RegExpExecArray | null;
            while ((match = regex.exec(text)) !== null) {
                const start = match.index;
                const end = start + match[0].length;
                // Skip if this region overlaps with an earlier match
                if (!results.some((r) => start >= r.start && start < r.end)) {
                    // Also try to consume a trailing :line or :line:col suffix
                    const remaining = text.slice(end);
                    const suffixMatch = remaining.match(/^:(\d+)(?::(\d+))?/);
                    const fullEnd = suffixMatch ? end + suffixMatch[0].length : end;
                    const fullText = text.slice(start, fullEnd);
                    results.push({ start, end: fullEnd, text: fullText });
                }
            }
        }
        return results;
    }
}

/**
 * Resolve a raw file path from terminal output and open it.
 * - Paths with :line suffix open in VS Code at that line.
 * - Absolute paths open in the native file explorer.
 * - Relative paths are resolved against the terminal CWD.
 */
export function makeFilePathHandler(getCwd: () => string | undefined): (rawPath: string) => void {
    return (rawPath: string) => {
        const lineMatch = rawPath.match(LINE_COL_SUFFIX);
        const path = lineMatch ? lineMatch[1] : rawPath;
        const line = lineMatch ? lineMatch[2] : undefined;

        let resolved = path;
        if (!isAbsolute(path)) {
            const cwd = getCwd();
            if (cwd) {
                resolved = cwd + "/" + path;
            }
        }

        // Normalize backslashes to forward slashes
        resolved = resolved.replace(/\\/g, "/");

        const api = window.api;
        if (!api) return;

        if (line) {
            // Open in VS Code at specific line
            api.openExternal(`vscode://file/${encodeURI(resolved)}:${line}`);
        } else {
            // Reveal in file explorer (Explorer/Finder/file manager)
            api.revealInFileExplorer(resolved);
        }
    };
}

function isAbsolute(path: string): boolean {
    // Unix absolute
    if (path.startsWith("/")) return true;
    // Windows absolute (C:\ or C:/)
    if (/^[A-Za-z]:[\\\/]/.test(path)) return true;
    // Home dir expansion
    if (path.startsWith("~/")) return true;
    return false;
}

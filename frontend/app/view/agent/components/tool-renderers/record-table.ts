// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Detect a "list of records" result — a top-level array of flat objects — so it
 * can render as a table instead of a JSON blob. Conservative on purpose: only a
 * top-level array whose every element is a flat object of scalar values
 * qualifies (no nested objects/arrays, bounded column count), so it can't fire
 * on shapes better left as JSON or handled by a named/terminal renderer.
 *
 * Registered by shape *below* the coarse-kind built-ins, so it only improves the
 * otherwise-JSON unknown-tool path (mcp__*, provider-specific, …).
 * See SPEC_TOOL_RESULT_RENDERER_REGISTRY_2026_06_17.md §5.3.
 */

const MAX_COLUMNS = 12;
const MAX_ROWS = 200;

export interface RecordTableData {
    columns: string[];
    rows: ReadonlyArray<Record<string, unknown>>;
    /** Rows dropped beyond MAX_ROWS. */
    truncatedRows: number;
}

function isScalar(v: unknown): boolean {
    return v === null || typeof v === "string" || typeof v === "number" || typeof v === "boolean";
}

function isFlatRecord(v: unknown): v is Record<string, unknown> {
    if (!v || typeof v !== "object" || Array.isArray(v)) return false;
    for (const val of Object.values(v as Record<string, unknown>)) {
        if (!isScalar(val)) return false;
    }
    return true;
}

/** Extract a record table from a result, or null if it isn't a flat record list. */
export function extractRecords(result: unknown): RecordTableData | null {
    if (!Array.isArray(result) || result.length === 0) return null;
    if (!result.every(isFlatRecord)) return null;

    // Column order = first-seen union of keys across rows.
    const columns: string[] = [];
    const seen = new Set<string>();
    for (const row of result as Record<string, unknown>[]) {
        for (const k of Object.keys(row)) {
            if (!seen.has(k)) {
                seen.add(k);
                columns.push(k);
            }
        }
    }
    if (columns.length === 0 || columns.length > MAX_COLUMNS) return null;

    const rows = (result as Record<string, unknown>[]).slice(0, MAX_ROWS);
    return { columns, rows, truncatedRows: Math.max(0, result.length - rows.length) };
}

export function looksLikeRecords(result: unknown): boolean {
    return extractRecords(result) != null;
}

/** Render a single cell value as text (empty for null/undefined). */
export function cellText(v: unknown): string {
    if (v == null) return "";
    const s = String(v);
    return s.length > 200 ? s.slice(0, 200) + "…" : s;
}

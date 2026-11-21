#!/usr/bin/env node
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Generates both sides' tables of deep-link destinations from the one registry
// (docs/specs/SPEC_DEEP_LINKS_ONE_FRAMEWORK_2026_11_20.md §4, R10 to R12, §10).
//
//   crates/common/deeplinks/destinations.json      the registry: every view, its places and parameters
//   -> frontend/app/deeplinks/registry.generated.ts  `DEEPLINK_REGISTRY`, read by the app's parser and executor
//   -> crates/common/src/deeplinks/registry.rs       `REGISTRY`, read by srv's and mcp's parser
//   -> docs/deep-links.md                            the reference page for the user documentation
//
// Usage:
//   node scripts/gen-deeplinks.mjs           write the three files
//   node scripts/gen-deeplinks.mjs --check   fail if any of them isn't what this would write (CI)
//
// It refuses to write anything from a registry it can't vouch for: a duplicate id, an unknown
// kind, effect, sensitivity or agent rule, a label key missing from the English dictionary, an
// alias whose target is not a place of the registry.
import { readFileSync, writeFileSync, existsSync, mkdirSync } from "node:fs";
import { dirname, join, relative } from "node:path";
import { fileURLToPath } from "node:url";
import { isMain } from "./lib/is-main.mjs";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
const SOURCE = join(ROOT, "crates/common/deeplinks/destinations.json");
const EN = join(ROOT, "frontend/app/i18n/locales/en.json");
const TS_OUT = join(ROOT, "frontend/app/deeplinks/registry.generated.ts");
const RS_OUT = join(ROOT, "crates/common/src/deeplinks/registry.rs");
const MD_OUT = join(ROOT, "docs/deep-links.md");

export const KINDS = ["enum", "id", "slug", "int", "opaque", "path", "name"];
export const EFFECTS = ["none", "starts", "network"];
export const SENSITIVITIES = ["public", "private", "secret", "path", "widget"];
export const AGENT_RULES = ["yes", "own", "reveal", "approved", "no"];
export const SURFACES = ["desktop", "web", "mobile"];
const VIEW_ID = /^[a-z][a-z0-9-]*$/;
const NAME = /^[a-z][a-z0-9_-]*$/;

/** Every problem in `reg`, as lines; empty when it is sound. `labels` is the set of English keys. */
export function validate(reg, labels) {
    const problems = [];
    const label = (where, key) => {
        if (typeof key !== "string" || !labels.has(key)) problems.push(`${where}: label "${key}" is not in en.json`);
    };
    if (reg.grammar !== 1) problems.push(`grammar must be 1 (the grammar is frozen, R9), not ${reg.grammar}`);
    if (!Number.isInteger(reg.build) || reg.build < 1) problems.push("build must be a positive integer");
    if (reg.maxBytes !== 512) problems.push("maxBytes must be 512 (R8)");
    if (typeof reg.separator !== "string" || [...reg.separator].length !== 1) problems.push("separator must be one character");
    const ids = new Set();
    for (const view of reg.views ?? []) {
        const at = `view ${view.id}`;
        if (!VIEW_ID.test(view.id ?? "")) problems.push(`${at}: id must match ${VIEW_ID}`);
        if (ids.has(view.id)) problems.push(`${at}: duplicate`);
        ids.add(view.id);
        label(at, view.label);
        if (typeof view.icon !== "string" || !view.icon) problems.push(`${at}: needs an icon`);
        if (!Number.isInteger(view.build) || view.build < 1) problems.push(`${at}: build must be a positive integer`);
        if (!EFFECTS.includes(view.effect)) problems.push(`${at}: effect "${view.effect}"`);
        if (!SENSITIVITIES.includes(view.sensitivity)) problems.push(`${at}: sensitivity "${view.sensitivity}"`);
        if (!AGENT_RULES.includes(view.agent)) problems.push(`${at}: agent "${view.agent}"`);
        if (view.effect !== "none" && view.agent !== "reveal") problems.push(`${at}: a worker (effect ${view.effect}) is reveal-only for an agent (G1)`);
        if (!Array.isArray(view.surfaces) || view.surfaces.some((s) => !SURFACES.includes(s))) problems.push(`${at}: surfaces`);
        if (typeof view.group !== "string") problems.push(`${at}: needs a group`);
        const placeIds = new Set();
        for (const place of view.places ?? []) {
            const pat = `${at} place ${place.id}`;
            if (!NAME.test(place.id ?? "")) problems.push(`${pat}: id`);
            if (placeIds.has(place.id)) problems.push(`${pat}: duplicate`);
            placeIds.add(place.id);
            if (!Array.isArray(place.segments) || place.segments.length === 0) problems.push(`${pat}: needs segments`);
            const enumOnly = place.segments?.length === 1 && place.segments[0].kind === "enum";
            if (!enumOnly) label(pat, place.label);
            const vars = new Set();
            for (const seg of place.segments ?? []) {
                if (typeof seg === "string") {
                    if (!NAME.test(seg)) problems.push(`${pat}: literal "${seg}"`);
                    continue;
                }
                if (!NAME.test(seg.var ?? "")) problems.push(`${pat}: var name "${seg.var}"`);
                if (vars.has(seg.var)) problems.push(`${pat}: var ${seg.var} twice`);
                vars.add(seg.var);
                checkKind(problems, `${pat} var ${seg.var}`, seg, label);
            }
            if (place.anchor) {
                if (place.anchor.kind !== "id") problems.push(`${pat}: an anchor is an id`);
                label(`${pat} anchor`, place.anchor.label);
            }
            const params = new Set();
            for (const p of place.params ?? []) {
                const pp = `${pat} param ${p.name}`;
                if (!/^[a-z][a-z0-9_]{0,31}$/.test(p.name ?? "")) problems.push(`${pp}: name`);
                if (params.has(p.name)) problems.push(`${pp}: duplicate`);
                params.add(p.name);
                if (["pane", "tab", "window", "target", "connection", "host"].includes(p.name)) {
                    problems.push(`${pp}: a link never names a target or a connection (§3.2, G7)`);
                }
                label(pp, p.label);
                checkKind(problems, pp, p, label);
                if (p.onlyWith) {
                    for (const [v, allowed] of Object.entries(p.onlyWith)) {
                        const seg = place.segments.find((s) => typeof s !== "string" && s.var === v);
                        if (!seg || seg.kind !== "enum") problems.push(`${pp}: onlyWith names ${v}, not an enum var of the place`);
                        else for (const a of allowed) if (!seg.values.some((x) => x.id === a)) problems.push(`${pp}: onlyWith ${v}=${a} is not a value`);
                    }
                }
            }
        }
    }
    for (const alias of reg.aliases ?? []) {
        const target = (reg.views ?? []).find((v) => v.id === alias.to?.[0]);
        if (!target) problems.push(`alias ${alias.from?.join("/")}: target view ${alias.to?.[0]} is not in the registry`);
        if (ids.has(alias.from?.[0])) problems.push(`alias ${alias.from?.join("/")}: shadows the view ${alias.from[0]}`);
        const fromVars = (alias.from ?? []).filter((s) => /^\{\w+\}$/.test(s));
        const toVars = (alias.to ?? []).filter((s) => /^\{\w+\}$/.test(s));
        if (fromVars.join() !== toVars.join()) problems.push(`alias ${alias.from?.join("/")}: from and to name different vars`);
    }
    for (const code of Object.keys(reg.errors ?? {})) if (!/^[a-z_]+$/.test(code)) problems.push(`error code ${code}`);
    return problems;
}

function checkKind(problems, at, spec, label) {
    if (!KINDS.includes(spec.kind)) {
        problems.push(`${at}: kind "${spec.kind}"`);
        return;
    }
    if (spec.kind === "enum") {
        if (!Array.isArray(spec.values) || spec.values.length === 0) problems.push(`${at}: an enum needs values`);
        const seen = new Set();
        for (const v of spec.values ?? []) {
            if (!/^[a-z0-9][a-z0-9-]*$/.test(v.id ?? "")) problems.push(`${at}: value "${v.id}"`);
            if (seen.has(v.id)) problems.push(`${at}: value ${v.id} twice`);
            seen.add(v.id);
            label(`${at} value ${v.id}`, v.label);
        }
    }
    if (spec.kind === "int") {
        if (!Number.isInteger(spec.min) || !Number.isInteger(spec.max) || spec.min > spec.max) problems.push(`${at}: an int needs min <= max`);
        if (spec.max > 2147483647 || spec.min < 0) problems.push(`${at}: an int is between 0 and 2^31-1`);
    }
}

const q = (s) => JSON.stringify(s);
const HEADER = (comment) => [
    `${comment} Copyright 2026, AgentMux Corp.`,
    `${comment} SPDX-License-Identifier: Apache-2.0`,
    "",
    `${comment} @generated by scripts/gen-deeplinks.mjs from crates/common/deeplinks/destinations.json. DO NOT EDIT:`,
    `${comment} change the registry, then run \`node scripts/gen-deeplinks.mjs\`.`,
];

/** The app's table: the registry as it is, with each label named `labelKey` (so the key check finds it). */
export function renderTs(reg) {
    const relabel = (value) => {
        if (Array.isArray(value)) return value.map(relabel);
        if (value && typeof value === "object") {
            return Object.fromEntries(
                Object.entries(value)
                    .filter(([k]) => k !== "$comment")
                    .map(([k, v]) => [k === "label" ? "labelKey" : k, relabel(v)])
            );
        }
        return value;
    };
    const body = JSON.stringify(relabel(reg), null, 4)
        // `labelKey: "a.b"` unquoted, as the dictionary's key check reads a table.
        .replace(/^(\s*)"([A-Za-z_$][\w$]*)":/gm, "$1$2:");
    return [
        ...HEADER("//"),
        "",
        'import type { DeepLinkRegistry } from "./registry-types";',
        "",
        "/** Every deep-link destination (R10). Read through `./registry`, never directly. */",
        `export const DEEPLINK_REGISTRY: DeepLinkRegistry = ${body};`,
        "",
    ].join("\n");
}

const rsStr = (s) => (s === undefined || s === null ? "None" : `Some(${q(s)})`);
const rsKind = (k) => `Kind::${k[0].toUpperCase()}${k.slice(1)}`;
const rsCap = (s) => s[0].toUpperCase() + s.slice(1);
const rsInt = (n) => (n === undefined ? "None" : `Some(${n})`);

function rsValues(values) {
    return `&[${(values ?? []).map((v) => `EnumValue { id: ${q(v.id)}, label_key: ${q(v.label)} }`).join(", ")}]`;
}

function rsSegment(seg) {
    if (typeof seg === "string") return `Segment::Lit(${q(seg)})`;
    return `Segment::Var(Var { name: ${q(seg.var)}, kind: ${rsKind(seg.kind)}, values: ${rsValues(seg.values)}, min: ${rsInt(seg.min)}, max: ${rsInt(seg.max)}, agent_own: ${!!seg.agentOwn} })`;
}

function rsParam(p) {
    const only = p.onlyWith ? Object.entries(p.onlyWith)[0] : null;
    const onlyWith = only ? `Some((${q(only[0])}, &[${only[1].map(q).join(", ")}]))` : "None";
    return `Param { name: ${q(p.name)}, kind: ${rsKind(p.kind)}, label_key: ${q(p.label)}, values: ${rsValues(p.values)}, min: ${rsInt(p.min)}, max: ${rsInt(p.max)}, only_with: ${onlyWith}, agent_own: ${!!p.agentOwn} }`;
}

function rsPlace(place, indent) {
    const i = " ".repeat(indent);
    const anchor = place.anchor ? `Some(Anchor { label_key: ${q(place.anchor.label)} })` : "None";
    return [
        `${i}Place {`,
        `${i}    id: ${q(place.id)},`,
        `${i}    label_key: ${rsStr(place.label)},`,
        `${i}    segments: &[${place.segments.map(rsSegment).join(", ")}],`,
        `${i}    sections: ${!!place.sections},`,
        `${i}    anchor: ${anchor},`,
        `${i}    params: &[${(place.params ?? []).map(rsParam).join(", ")}],`,
        `${i}},`,
    ].join("\n");
}

/** srv's and mcp's table: the same registry as typed statics. */
export function renderRs(reg) {
    const views = reg.views.map((v) =>
        [
            "    View {",
            `        id: ${q(v.id)},`,
            `        pane_view: ${v.paneView === null ? "None" : rsStr(v.paneView ?? v.id)},`,
            `        label_key: ${q(v.label)},`,
            `        icon: ${q(v.icon)},`,
            `        build: ${v.build},`,
            `        effect: Effect::${rsCap(v.effect)},`,
            `        sensitivity: Sensitivity::${rsCap(v.sensitivity)},`,
            `        agent: AgentRule::${rsCap(v.agent)},`,
            `        surfaces: &[${v.surfaces.map(q).join(", ")}],`,
            `        command: ${rsStr(v.command)},`,
            `        prefix: ${!!v.prefix},`,
            `        requires_place: ${!!v.requiresPlace},`,
            "        places: &[",
            ...v.places.map((p) => rsPlace(p, 12)),
            "        ],",
            "    },",
        ].join("\n")
    );
    const aliases = (reg.aliases ?? []).map((a) => `    Alias { from: &[${a.from.map(q).join(", ")}], to: &[${a.to.map(q).join(", ")}] },`);
    const unlinked = (reg.unlinked ?? []).map((u) => `    Unlinked { view: ${q(u.view)}, reason: ${q(u.reason)} },`);
    const errors = Object.entries(reg.errors ?? {}).map(([k, v]) => `    (${q(k)}, ${q(v)}),`);
    return [
        ...HEADER("//"),
        "",
        "use super::types::*;",
        "",
        "/// Every deep-link destination (R10). Read through `super`, never directly.",
        "pub static REGISTRY: Registry = Registry {",
        `    grammar: ${reg.grammar},`,
        `    build: ${reg.build},`,
        `    max_bytes: ${reg.maxBytes},`,
        `    separator: ${q(reg.separator)},`,
        "    views: VIEWS,",
        "    aliases: ALIASES,",
        "    unlinked: UNLINKED,",
        "    errors: ERRORS,",
        "};",
        "",
        "const VIEWS: &[View] = &[",
        ...views,
        "];",
        "",
        "const ALIASES: &[Alias] = &[",
        ...aliases,
        "];",
        "",
        "const UNLINKED: &[Unlinked] = &[",
        ...unlinked,
        "];",
        "",
        "const ERRORS: &[(&str, &str)] = &[",
        ...errors,
        "];",
        "",
    ].join("\n");
}

function shapeOf(view, place) {
    const head = view.prefix ? `${view.id}.<package>` : view.id;
    const segs = place
        ? place.segments.map((s) => (typeof s === "string" ? s : s.kind === "enum" ? `<${s.var}>` : `<${s.var}: ${s.kind}>`))
        : [];
    const params = (place?.params ?? []).map((p) => `${p.name}=<${p.kind}>`);
    return `agentmux://${[head, ...segs].join("/")}${params.length ? `?${params.join("&")}` : ""}${place?.anchor ? "#<id>" : ""}`;
}

const AGENT_WORDS = {
    yes: "yes",
    own: "its own place only",
    reveal: "reveals an open pane only",
    approved: "an approved package only",
    no: "no",
};

/** The reference page (§10): the grammar, then one table per view. */
export function renderMd(reg, en) {
    const t = (k) => en[k] ?? k;
    const lines = [
        "<!-- @generated by scripts/gen-deeplinks.mjs from crates/common/deeplinks/destinations.json. DO NOT EDIT. -->",
        "",
        "# Deep links",
        "",
        "**Status:** living (generated from the registry; it changes with every destination added)",
        "",
        "A deep link opens a view of AgentMux at a place inside it: a Settings section, a Kanban task, a file at a line.",
        "Every link has the same form:",
        "",
        "```",
        "agentmux://view[/place][?key=value&…][#anchor]",
        "```",
        "",
        "- Every part is an id, never a label, so a renamed label does not break a link.",
        "- A link has one written form: lower-case ids, upper-case percent-encoding, parameters in the order shown here, no trailing slash.",
        `- A link is at most ${reg.maxBytes} bytes, is percent-decoded once, and names each parameter once.`,
        "- A link never names a pane, tab or window, and never opens a connection.",
        "- A link with a view, place or parameter this AgentMux does not have is refused with the reason, never partly opened.",
        "",
    ];
    for (const view of reg.views) {
        lines.push(`## ${t(view.label)} (\`${view.id}\`)`, "");
        const notYet = view.build > reg.build;
        lines.push(
            `Agents: ${AGENT_WORDS[view.agent]}.${view.surfaces.length ? ` Surfaces: ${view.surfaces.join(", ")}.` : ""}${notYet ? " Not yet available: this view is declared for a later build." : ""}`,
            ""
        );
        lines.push("| Link | Opens |", "|---|---|");
        if (!view.requiresPlace) lines.push(`| \`${shapeOf(view, null)}\` | ${t(view.label)} |`);
        for (const place of view.places) {
            const enumSeg = place.segments.find((s) => typeof s !== "string" && s.kind === "enum");
            const what = place.label ? t(place.label) : enumSeg ? enumSeg.values.map((v) => `\`${v.id}\` (${t(v.label)})`).join(", ") : place.id;
            const own = place.segments.some((s) => typeof s !== "string" && s.agentOwn) || (place.params ?? []).some((p) => p.agentOwn);
            lines.push(`| \`${shapeOf(view, place)}\` | ${what}${own ? ". An agent may name only itself" : ""} |`);
        }
        lines.push("");
    }
    if (reg.aliases?.length) {
        lines.push("## Other written forms", "", "| Link | Is the same as |", "|---|---|");
        for (const a of reg.aliases) lines.push(`| \`agentmux://${a.from.join("/")}\` | \`agentmux://${a.to.join("/")}\` |`);
        lines.push("");
    }
    return lines.join("\n");
}

export function load() {
    const reg = JSON.parse(readFileSync(SOURCE, "utf8"));
    const en = JSON.parse(readFileSync(EN, "utf8"));
    return { reg, en };
}

export function outputs(reg, en) {
    return [
        [TS_OUT, renderTs(reg)],
        [RS_OUT, renderRs(reg)],
        [MD_OUT, renderMd(reg, en)],
    ];
}

function main() {
    const check = process.argv.includes("--check");
    const { reg, en } = load();
    const problems = validate(reg, new Set(Object.keys(en)));
    if (problems.length) {
        console.error(`gen-deeplinks: the registry is not sound:\n  ${problems.join("\n  ")}`);
        process.exit(1);
    }
    let stale = 0;
    for (const [path, text] of outputs(reg, en)) {
        const now = existsSync(path) ? readFileSync(path, "utf8").replace(/\r\n/g, "\n") : null;
        if (now === text) continue;
        if (check) {
            console.error(`gen-deeplinks: ${relative(ROOT, path)} is stale: run node scripts/gen-deeplinks.mjs`);
            stale++;
        } else {
            mkdirSync(dirname(path), { recursive: true });
            writeFileSync(path, text);
            console.log(`gen-deeplinks: wrote ${relative(ROOT, path)}`);
        }
    }
    if (stale) process.exit(1);
}

if (isMain(import.meta.url)) main();

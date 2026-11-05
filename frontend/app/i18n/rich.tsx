// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// A translated string with a little inline markup: `<strong>`, `<code>` and `<em>`, one level deep. The
// markup is read from the dictionary's template before any `{{name}}` is filled in, and what fills a name is
// plain text, so a value such as an email address can never become markup. Nothing is passed to innerHTML.
import { For, type JSX } from "solid-js";
import { appLocale, t } from "./i18n";
import { interpolate, type Params } from "./translate";

export type RichTag = "strong" | "code" | "em";

export interface RichPart {
    tag?: RichTag;
    text: string;
}

const MARKUP = /<(strong|code|em)>([\s\S]*?)<\/\1>/g;

/** Splits a template into plain and tagged pieces; any other `<` stays text. */
export function richParts(template: string): RichPart[] {
    const parts: RichPart[] = [];
    let last = 0;
    for (const m of template.matchAll(MARKUP)) {
        if (m.index > last) parts.push({ text: template.slice(last, m.index) });
        parts.push({ tag: m[1] as RichTag, text: m[2] });
        last = m.index + m[0].length;
    }
    if (last < template.length) parts.push({ text: template.slice(last) });
    return parts;
}

/** The text of `key` with its markup rendered and `params` filled in. Reactive, like `t`. */
export function Rich(props: { k: string; params?: Params }): JSX.Element {
    const parts = () => richParts(t(props.k)).map((p) => ({ ...p, text: interpolate(p.text, props.params, appLocale()) }));
    return (
        <For each={parts()}>
            {(p) => {
                if (p.tag === "strong") return <strong>{p.text}</strong>;
                if (p.tag === "code") return <code>{p.text}</code>;
                if (p.tag === "em") return <em>{p.text}</em>;
                return <>{p.text}</>;
            }}
        </For>
    );
}

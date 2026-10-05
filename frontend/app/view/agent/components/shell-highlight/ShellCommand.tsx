// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * ShellCommand — a Bash-tool command with syntax colours that are there on
 * first paint (no async grammar, so no flash) and follow the active theme.
 *
 * POSIX commands go through `tokenizeShell`; PowerShell keeps the Shiki
 * grammar until it has its own tokenizer; cmd renders as plain text.
 * The text is never altered: every span is a range of the original command.
 *
 * A heredoc body whose language can be told (embedded.ts) is highlighted as
 * that language: a shell body with the same tokenizer, on first paint; any
 * other language by Shiki, after a plain first paint. Unknown bodies stay
 * plain.
 *
 * Spec: docs/specs/SPEC_AGENT_PANE_BASH_HIGHLIGHTING_2026_10_04.md §3
 */

import { For, Match, Show, Switch, createMemo, createSignal, onCleanup, type JSX } from "solid-js";
import { HighlightedCode } from "../HighlightedCode";
import { SHELL_LANG, embeddedBodies } from "./embedded";
import { cachedRuns, highlightBody } from "./embedded-highlight";
import { detectShellFlavor } from "./flavor";
import { MAX_TOKENIZE_CHARS, tokenizeShell, type Token } from "./tokenize";

const CACHE_LIMIT = 200;
const tokenCache = new Map<string, Token[]>();

function tokensFor(command: string): Token[] {
    const hit = tokenCache.get(command);
    if (hit) {
        // Refresh recency.
        tokenCache.delete(command);
        tokenCache.set(command, hit);
        return hit;
    }
    const { tokens } = tokenizeShell(command);
    // The key is the whole command: caching a huge one (a big heredoc, a base64
    // payload) would keep it alive after its panel is gone.
    if (command.length > MAX_TOKENIZE_CHARS) return tokens;
    tokenCache.set(command, tokens);
    if (tokenCache.size > CACHE_LIMIT) {
        const oldest = tokenCache.keys().next().value;
        if (oldest !== undefined) tokenCache.delete(oldest);
    }
    return tokens;
}

/** One token of `source` as text or a class-coloured span. */
function renderToken(source: string, t: Token): JSX.Element {
    const text = source.slice(t.start, t.end);
    return t.kind === "plain" ? text : <span class={`sh-${t.kind}`}>{text}</span>;
}

/** A heredoc body in a known language. Renders exactly `text`. */
const EmbeddedBody = (props: { text: string; lang: string }): JSX.Element => {
    // Each body token gets its own instance, so its text and language are fixed.
    const { text, lang } = props;
    if (lang === SHELL_LANG) {
        return (
            <span class="sh-embedded">
                <For each={tokensFor(text)}>{(t) => renderToken(text, t)}</For>
            </span>
        );
    }
    const [runs, setRuns] = createSignal(cachedRuns(text, lang));
    if (!runs()) {
        let live = true;
        onCleanup(() => (live = false));
        void highlightBody(text, lang).then((r) => {
            if (live && r) setRuns(r);
        });
    }
    return (
        <Show when={runs()} fallback={<span class="sh-heredoc-body">{text}</span>}>
            {(r) => (
                <span class="sh-embedded sh-shiki">
                    <For each={r()}>{(run) => (run.style ? <span style={run.style}>{run.text}</span> : run.text)}</For>
                </span>
            )}
        </Show>
    );
};

interface ShellCommandProps {
    command: string;
    /** Extra CSS class applied to the outer <pre>. */
    class?: string;
}

/**
 * A POSIX command's token spans with no wrapper, for inline use: the
 * collapsed tool row and the peek popover. Any other flavor is plain text.
 * `embedded` also highlights heredoc bodies in their own language; the row
 * leaves it off, so a transcript of rows never loads Shiki.
 */
export const ShellTokens = (props: { command: string; embedded?: boolean }): JSX.Element => {
    const command = () => props.command ?? "";
    const posix = createMemo(() => detectShellFlavor(command()) === "posix");
    const tokens = createMemo(() => (posix() ? tokensFor(command()) : []));
    // Heredoc bodies with a known language, by start offset.
    const bodies = createMemo(() =>
        props.embedded
            ? new Map(embeddedBodies(tokens(), command()).map((b) => [b.start, b.lang] as const))
            : new Map<number, string>()
    );
    return (
        <Show when={posix()} fallback={command()}>
            <For each={tokens()}>
                {(t) => {
                    const lang = t.kind === "heredoc-body" ? bodies().get(t.start) : undefined;
                    return lang ? (
                        <EmbeddedBody text={command().slice(t.start, t.end)} lang={lang} />
                    ) : (
                        renderToken(command(), t)
                    );
                }}
            </For>
        </Show>
    );
};

ShellTokens.displayName = "ShellTokens";

export const ShellCommand = (props: ShellCommandProps): JSX.Element => {
    // A malformed tool call can arrive without a command.
    const command = () => props.command ?? "";
    const flavor = createMemo(() => detectShellFlavor(command()));
    const cls = () => `agent-highlighted-code agent-shell-command${props.class ? ` ${props.class}` : ""}`;

    return (
        <Switch>
            <Match when={flavor() === "powershell"}>
                <HighlightedCode code={command()} lang="powershell" class={props.class} />
            </Match>
            <Match when={flavor() === "cmd"}>
                <pre class={cls()}>{command()}</pre>
            </Match>
            <Match when={true}>
                <pre class={cls()}>
                    <ShellTokens command={command()} embedded />
                </pre>
            </Match>
        </Switch>
    );
};

ShellCommand.displayName = "ShellCommand";

/** The `$ command` header shared by the finished and the streaming Bash panel. */
export const BashCommandView = (props: { command: string }): JSX.Element => (
    <div class="agent-bash-cmd">
        <span class="agent-bash-dollar">$</span>
        <ShellCommand command={props.command} class="agent-bash-cmd-code" />
    </div>
);

BashCommandView.displayName = "BashCommandView";

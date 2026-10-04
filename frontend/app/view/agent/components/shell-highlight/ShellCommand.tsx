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
 * Spec: docs/specs/SPEC_AGENT_PANE_BASH_HIGHLIGHTING_2026_10_04.md §3
 */

import { For, Match, Switch, createMemo, type JSX } from "solid-js";
import { HighlightedCode } from "../HighlightedCode";
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

interface ShellCommandProps {
    command: string;
    /** Extra CSS class applied to the outer <pre>. */
    class?: string;
}

export const ShellCommand = (props: ShellCommandProps): JSX.Element => {
    // A malformed tool call can arrive without a command.
    const command = () => props.command ?? "";
    const flavor = createMemo(() => detectShellFlavor(command()));
    const tokens = createMemo(() => (flavor() === "posix" ? tokensFor(command()) : []));
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
                    <For each={tokens()}>
                        {(t) => {
                            const text = command().slice(t.start, t.end);
                            return t.kind === "plain" ? text : <span class={`sh-${t.kind}`}>{text}</span>;
                        }}
                    </For>
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

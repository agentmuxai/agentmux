// Copyright 2025, Command Line Inc.
// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** The find bar's markup over a state from search-state.ts: the input, the result count and the buttons. */

import clsx from "clsx";
import { Show, type JSX } from "solid-js";

import { DeclIconButton } from "@/app/element/ui/DeclIconButton";
import { Input } from "./input";
import type { SearchState } from "./search-state";

function AdditionalButtons(props: { s: SearchState }): JSX.Element {
    const s = props.s;
    return (
        <Show when={s.caseSensitiveDecl || s.wholeWordDecl || s.regexDecl}>
            <div class="additional-buttons">
                <Show when={s.caseSensitiveDecl}>
                    <DeclIconButton decl={s.caseSensitiveDecl} />
                </Show>
                <Show when={s.wholeWordDecl}>
                    <DeclIconButton decl={s.wholeWordDecl} />
                </Show>
                <Show when={s.regexDecl}>
                    <DeclIconButton decl={s.regexDecl} />
                </Show>
            </div>
        </Show>
    );
}

export function SearchBar(props: { s: SearchState }): JSX.Element {
    const s = props.s;
    return (
        <div class="search-container" style={s.style()} ref={s.register}>
            <Input
                placeholder="Search"
                value={s.searchValue()}
                onChange={s.setSearch}
                onKeyDown={s.onKeyDown}
                autoFocus
                autoSelect
            />
            <div
                class={clsx("search-results", { hidden: s.resultsCount() === 0 })}
                aria-live="polite"
                aria-label="Search Results"
            >
                {s.resultsIndex() + 1}/{s.resultsCount()}
            </div>

            <AdditionalButtons s={s} />

            <div class="right-buttons">
                <DeclIconButton decl={s.prevDecl} />
                <DeclIconButton decl={s.nextDecl} />
                <DeclIconButton decl={s.closeDecl} />
            </div>
        </div>
    );
}

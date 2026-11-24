// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** The Large text viewer's find strip: a text, next and previous, and what the search found. */

import { t } from "@/app/i18n/i18n";
import { tip } from "@/app/element/tip";
import { Button, CloseButton, FilterInput, IconButton } from "@/app/element/ui";
import { keyLabel } from "@/app/keybindings";
import { Show, type JSX } from "solid-js";
import type { LargeFind } from "./large-find-state";

export function LargeFindBar(props: { find: LargeFind }): JSX.Element {
    const f = props.find;
    const closeButton = () => <CloseButton label="Close find" onClick={f.close} />;
    const idle = () => f.canSearch() && !f.error() && !f.status();
    const onKeyDown = (e: KeyboardEvent) => {
        if (e.key === "Enter") {
            e.preventDefault();
            if (e.shiftKey) f.previous();
            else f.next();
        } else if (e.key === "Escape") {
            e.preventDefault();
            e.stopPropagation();
            f.close();
        }
    };
    return (
        <div class="large-text-find" role="search" aria-label="Find in the file">
            <FilterInput
                class="large-text-find-input"
                value={f.text()}
                onInput={f.edit}
                placeholder="Find text"
                ariaLabel="Find in the file"
                inputRef={f.bindInput}
                onKeyDown={onKeyDown}
                testId="large-text-find"
            />
            <div class="large-text-find-nav">
                <Button
                    density="compact"
                    class="large-text-find-toggle"
                    pressed={f.matchCase()}
                    aria-label={t("editor.find.matchCase")}
                    {...tip(t("editor.find.matchCaseTip"))}
                    onClick={f.toggleCase}
                >
                    {t("editor.find.matchCaseShort")}
                </Button>
                <Button
                    density="compact"
                    class="large-text-find-toggle"
                    pressed={f.regex()}
                    aria-label={t("editor.find.regex")}
                    {...tip(t("editor.find.regexTip"))}
                    onClick={f.toggleRegex}
                >
                    {t("editor.find.regexShort")}
                </Button>
                <IconButton icon="chevron-up" density="compact" label="Previous match (Shift+Enter)" disabled={!f.canSearch()} onClick={f.previous} />
                <IconButton icon="chevron-down" density="compact" label="Next match (Enter)" disabled={!f.canSearch()} onClick={f.next} />
                {/* Nothing to say yet: the close button travels with the arrows, so a narrow strip never leaves it alone. */}
                <Show when={idle()}>{closeButton()}</Show>
            </div>
            <Show when={!idle()}>
                <div class="large-text-find-tail">
                    <span
                        class="large-text-find-status"
                        role="status"
                        classList={{ "large-text-find-error": !!f.error() }}
                        {...tip(`Find (${keyLabel("mod+f")})`)}
                    >
                        <Show when={f.canSearch()} fallback="A file dropped in has no path on disk to search">
                            {f.error() || f.status()}
                        </Show>
                    </span>
                    {closeButton()}
                </div>
            </Show>
        </div>
    );
}

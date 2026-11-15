// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** The hex viewer's find strip: a pattern, its mode, next and previous, and what the search found. */

import { tip } from "@/app/element/tip";
import { CloseButton, FilterInput, IconButton, SegmentedControl } from "@/app/element/ui";
import { keyLabel } from "@/app/keybindings";
import { Show, type JSX } from "solid-js";
import { FIND_MODES, type FindMode } from "./hex-find";
import type { HexFind } from "./hex-find-state";

export function HexFindBar(props: { find: HexFind }): JSX.Element {
    const f = props.find;
    const onKeyDown = (e: KeyboardEvent) => {
        if (e.key === "Enter") {
            e.preventDefault();
            if (e.shiftKey) f.previous();
            else f.next();
        } else if (e.key === "Escape") {
            e.preventDefault();
            f.close();
        }
    };
    return (
        <div class="hex-find" role="search" aria-label="Find in the file">
            <FilterInput
                class="hex-find-input"
                value={f.text()}
                onInput={f.edit}
                placeholder={f.mode() === "hex" ? "Find bytes: DE AD ?? EF" : "Find text"}
                ariaLabel="Find in the file"
                inputRef={f.bindInput}
                onKeyDown={onKeyDown}
                testId="hex-find"
            />
            <SegmentedControl<FindMode>
                density="compact"
                ariaLabel="What the pattern is"
                options={FIND_MODES.map((m) => ({ value: m.value, label: m.label }))}
                value={f.mode()}
                onChange={f.setMode}
            />
            <IconButton
                icon="chevron-up"
                density="compact"
                label={`Previous match (Shift+Enter)`}
                disabled={!f.canSearch()}
                onClick={f.previous}
            />
            <IconButton
                icon="chevron-down"
                density="compact"
                label={`Next match (Enter)`}
                disabled={!f.canSearch()}
                onClick={f.next}
            />
            <div class="hex-find-tail" classList={{ "hex-find-tail-idle": f.canSearch() && !f.error() && !f.status() }}>
                <span
                    class="hex-find-status"
                    role="status"
                    classList={{ "hex-find-error": !!f.error() }}
                    {...tip(`Find (${keyLabel("mod+f")})`)}
                >
                    <Show when={f.canSearch()} fallback="A file dropped in has no path on disk to search">
                        {f.error() || f.status()}
                    </Show>
                </span>
                <CloseButton label="Close find" onClick={f.close} />
            </div>
        </div>
    );
}

// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

import { atoms } from "@/app/store/global";
import { isBlank, makeIconClass } from "@/util/util";
import { computePosition, offset } from "@floating-ui/dom";
import clsx from "clsx";
import {
    createEffect,
    createMemo,
    createSignal,
    For,
    onCleanup,
    onMount,
    Show,
} from "solid-js";
import type { Accessor, JSX } from "solid-js";

interface SuggestionControlProps {
    anchorRef: { current: HTMLElement | null };
    isOpen: boolean;
    onClose: () => void;
    onSelect: (item: SuggestionType, queryStr: string) => boolean;
    onTab?: (item: SuggestionType, queryStr: string) => string;
    fetchSuggestions: SuggestionsFnType;
    className?: string;
    placeholderText?: string;
    children?: JSX.Element;
}

type BlockHeaderSuggestionControlProps = Omit<SuggestionControlProps, "anchorRef" | "isOpen"> & {
    blockRef: { current: HTMLElement | null };
    openAtom: Accessor<boolean>;
};

function SuggestionControl(props: SuggestionControlProps): JSX.Element {
    return (
        <Show when={props.isOpen && props.anchorRef.current != null && props.fetchSuggestions != null}>
            <SuggestionControlInner {...props} />
        </Show>
    );
}

function highlightPositions(target: string, positions: number[]): JSX.Element[] {
    if (target == null) return [];
    if (positions == null) return [<span>{target}</span>];
    const result: JSX.Element[] = [];
    let targetIndex = 0;
    let posIndex = 0;
    while (targetIndex < target.length) {
        if (posIndex < positions.length && targetIndex === positions[posIndex]) {
            result.push(
                <span class="text-blue-500 font-bold">{target[targetIndex]}</span>
            );
            posIndex++;
        } else {
            result.push(<span>{target[targetIndex]}</span>);
        }
        targetIndex++;
    }
    return result;
}

function getMimeTypeIconAndColor(fullConfig: FullConfigType, mimeType: string): [string, string] {
    if (mimeType == null) return [null, null];
    while (mimeType.length > 0) {
        const icon = fullConfig.mimetypes?.[mimeType]?.icon ?? null;
        const iconColor = fullConfig.mimetypes?.[mimeType]?.color ?? null;
        if (icon != null) return [icon, iconColor];
        mimeType = mimeType.slice(0, -1);
    }
    return [null, null];
}

function SuggestionIcon(props: { suggestion: SuggestionType }): JSX.Element {
    if (props.suggestion.iconsrc) {
        return <img src={props.suggestion.iconsrc} alt="favicon" class="w-4 h-4 object-contain" />;
    }
    if (props.suggestion.icon) {
        const iconClass = makeIconClass(props.suggestion.icon, true);
        return <i class={iconClass} style={{ color: props.suggestion.iconcolor }} />;
    }
    if (props.suggestion.type === "url") {
        const iconClass = makeIconClass("globe", true);
        return <i class={iconClass} style={{ color: props.suggestion.iconcolor }} />;
    } else if (props.suggestion.type === "file") {
        const fullConfig = atoms.fullConfigAtom();
        let icon: string = null;
        let iconColor: string = null;
        if (icon == null && props.suggestion["file:mimetype"] != null) {
            [icon, iconColor] = getMimeTypeIconAndColor(fullConfig, props.suggestion["file:mimetype"]);
        }
        const iconClass = makeIconClass(icon, true, { defaultIcon: "file" });
        return <i class={iconClass} style={{ color: iconColor }} />;
    }
    const iconClass = makeIconClass("file", true);
    return <i class={iconClass} />;
}

function SuggestionContent(props: { suggestion: SuggestionType }): JSX.Element {
    if (!isBlank(props.suggestion.subtext)) {
        return (
            <div class="flex flex-col">
                <div class="truncate text-white">{highlightPositions(props.suggestion.display, props.suggestion.matchpos)}</div>
                <div class="truncate text-sm text-secondary">
                    {highlightPositions(props.suggestion.subtext, props.suggestion.submatchpos)}
                </div>
            </div>
        );
    }
    return <span class="truncate">{highlightPositions(props.suggestion.display, props.suggestion.matchpos)}</span>;
}

function BlockHeaderSuggestionControl(props: BlockHeaderSuggestionControlProps): JSX.Element {
    const [headerElem, setHeaderElem] = createSignal<HTMLElement | null>(null);
    const isOpen = props.openAtom;

    createEffect(() => {
        const blockEl = props.blockRef.current;
        if (blockEl == null) {
            setHeaderElem(null);
            return;
        }
        const el = blockEl.querySelector("[data-role='block-header']");
        setHeaderElem(el as HTMLElement);
    });

    const newClass = clsx(props.className, "rounded-t-none");
    const anchorRef = createMemo(() => ({ current: headerElem() }));

    return (
        <SuggestionControl
            {...props}
            anchorRef={anchorRef()}
            isOpen={isOpen()}
            className={newClass}
        />
    );
}

const SuggestionControlNoResults = (props: { children?: JSX.Element }): JSX.Element => (
    <div class="flex items-center justify-center min-h-[120px] p-4">
        {props.children ?? <span class="text-gray-500">No Suggestions</span>}
    </div>
);

const SuggestionControlNoData = (props: { children?: JSX.Element }): JSX.Element => (
    <div class="flex items-center justify-center min-h-[120px] p-4">
        {props.children ?? <span class="text-gray-500">No Suggestions</span>}
    </div>
);

interface SuggestionControlInnerProps extends Omit<SuggestionControlProps, "isOpen"> {}

function SuggestionControlInner(props: SuggestionControlInnerProps): JSX.Element {
    const widgetId = crypto.randomUUID();
    const [query, setQuery] = createSignal("");
    const [suggestions, setSuggestions] = createSignal<SuggestionType[]>([]);
    const [selectedIndex, setSelectedIndex] = createSignal(0);
    const [fetched, setFetched] = createSignal(false);
    const [floatingStyle, setFloatingStyle] = createSignal<{ top: string; left: string }>({ top: "0px", left: "0px" });

    let reqNum = 0;
    let inputRef!: HTMLInputElement;
    let dropdownRef!: HTMLDivElement;
    let floatingRef!: HTMLDivElement;

    // Position floating element relative to anchor
    const updatePosition = async () => {
        if (!props.anchorRef.current || !floatingRef) return;
        const pos = await computePosition(props.anchorRef.current, floatingRef, {
            placement: "bottom",
            strategy: "absolute",
            middleware: [offset(-1)],
        });
        setFloatingStyle({ top: `${pos.y}px`, left: `${pos.x}px` });
    };

    onMount(() => {
        updatePosition();
        inputRef?.focus();
    });

    createEffect(() => {
        if (props.anchorRef.current) {
            updatePosition();
        }
    });

    // Fetch suggestions when query changes
    createEffect(() => {
        const q = query();
        reqNum++;
        const curReqNum = reqNum;
        props.fetchSuggestions(q, { widgetid: widgetId, reqnum: curReqNum }).then((results) => {
            if (results.reqnum !== curReqNum) return;
            setSuggestions(results.suggestions ?? []);
            setFetched(true);
        });
    });

    onCleanup(() => {
        reqNum++;
        props.fetchSuggestions("", { widgetid: widgetId, reqnum: reqNum, dispose: true });
    });

    // Click outside to close
    const handleClickOutside = (event: MouseEvent) => {
        if (floatingRef && !floatingRef.contains(event.target as Node)) {
            props.onClose();
        }
    };

    onMount(() => {
        document.addEventListener("mousedown", handleClickOutside);
        onCleanup(() => document.removeEventListener("mousedown", handleClickOutside));
    });

    // Scroll selected item into view
    createEffect(() => {
        const idx = selectedIndex();
        if (dropdownRef) {
            const children = dropdownRef.children;
            if (children[idx]) {
                (children[idx] as HTMLElement).scrollIntoView({ behavior: "auto", block: "nearest" });
            }
        }
    });

    const handleKeyDown = (e: KeyboardEvent) => {
        if (e.key === "ArrowDown") {
            e.preventDefault();
            e.stopPropagation();
            setSelectedIndex((prev) => Math.min(prev + 1, suggestions().length - 1));
        } else if (e.key === "ArrowUp") {
            e.preventDefault();
            e.stopPropagation();
            setSelectedIndex((prev) => Math.max(prev - 1, 0));
        } else if (e.key === "Enter") {
            e.preventDefault();
            e.stopPropagation();
            const idx = selectedIndex();
            let suggestion: SuggestionType = null;
            if (idx >= 0 && idx < suggestions().length) {
                suggestion = suggestions()[idx];
            }
            if (props.onSelect(suggestion, query())) {
                props.onClose();
            }
        } else if (e.key === "Escape") {
            e.preventDefault();
            e.stopPropagation();
            props.onClose();
        } else if (e.key === "Tab") {
            e.preventDefault();
            e.stopPropagation();
            const suggestion = suggestions()[selectedIndex()];
            if (suggestion != null) {
                const tabResult = props.onTab?.(suggestion, query());
                if (tabResult != null) setQuery(tabResult);
            }
        } else if (e.key === "PageDown") {
            e.preventDefault();
            e.stopPropagation();
            setSelectedIndex((prev) => Math.min(prev + 10, suggestions().length - 1));
        } else if (e.key === "PageUp") {
            e.preventDefault();
            e.stopPropagation();
            setSelectedIndex((prev) => Math.max(prev - 10, 0));
        }
    };

    return (
        <div
            ref={floatingRef!}
            class={clsx(
                "w-96 rounded-lg bg-modalbg shadow-lg border border-gray-700 z-[var(--zindex-typeahead-modal)] absolute",
                props.className
            )}
            style={{ position: "absolute", top: floatingStyle().top, left: floatingStyle().left }}
        >
            <div class="p-2">
                <input
                    ref={inputRef!}
                    type="text"
                    value={query()}
                    onChange={(e) => {
                        setQuery(e.target.value);
                        setSelectedIndex(0);
                    }}
                    onKeyDown={handleKeyDown}
                    class="w-full bg-gray-900 text-gray-100 px-4 py-2 rounded-md border border-gray-700 focus:outline-none focus:border-accent placeholder-secondary"
                    placeholder={props.placeholderText}
                />
            </div>
            <Show when={fetched()}>
                <Show
                    when={suggestions().length > 0}
                    fallback={
                        <div class="flex items-center justify-center min-h-[120px] p-4">
                            <Show
                                when={query() !== ""}
                                fallback={<SuggestionControlNoData />}
                            >
                                <SuggestionControlNoResults />
                            </Show>
                        </div>
                    }
                >
                    <div ref={dropdownRef!} class="max-h-96 overflow-y-auto divide-y divide-gray-700">
                        <For each={suggestions()}>
                            {(suggestion, index) => (
                                <div
                                    class={clsx(
                                        "flex items-center gap-3 px-4 py-2 cursor-pointer",
                                        index() === selectedIndex() ? "bg-accentbg" : "hover:bg-hoverbg",
                                        "text-gray-100"
                                    )}
                                    onClick={() => {
                                        props.onSelect(suggestion, query());
                                        props.onClose();
                                    }}
                                >
                                    <SuggestionIcon suggestion={suggestion} />
                                    <SuggestionContent suggestion={suggestion} />
                                </div>
                            )}
                        </For>
                    </div>
                </Show>
            </Show>
        </div>
    );
}

export { BlockHeaderSuggestionControl, SuggestionControl, SuggestionControlNoData, SuggestionControlNoResults };

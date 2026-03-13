// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

import { createSignalAtom, type SignalAtom } from "@/util/util";
import {
    autoUpdate,
    computePosition,
    offset,
    type Middleware,
} from "@floating-ui/dom";
import clsx from "clsx";
import { createEffect, createMemo, JSX, onCleanup, onMount, Show } from "solid-js";
import { Portal } from "solid-js/web";
import { IconButton, ToggleIconButton } from "./iconbutton";
import { Input } from "./input";
import "./search.scss";

type SearchProps = SearchAtoms & {
    anchorRef?: { current: HTMLElement | null };
    offsetX?: number;
    offsetY?: number;
    onSearch?: (search: string) => void;
    onNext?: () => void;
    onPrev?: () => void;
};

const SearchComponent = (props: SearchProps): JSX.Element => {
    const searchAtom = props.searchValue;
    const indexAtom = props.resultsIndex;
    const numResultsAtom = props.resultsCount;
    const isOpenAtom = props.isOpen;
    const regexAtom = props.regex;
    const wholeWordAtom = props.wholeWord;
    const caseSensitiveAtom = props.caseSensitive;

    const offsetX = props.offsetX ?? 10;
    const offsetY = props.offsetY ?? 10;

    const floatingStyleAtom = createSignalAtom("position:absolute;left:0px;top:0px");
    let floatingEl: HTMLElement | null = null;
    let cleanupAutoUpdate: (() => void) | null = null;

    const offsetCallback = ({ rects }: { rects: any }) => {
        const docRect = document.documentElement.getBoundingClientRect();
        let yOffsetCalc = -rects.floating.height - offsetY;
        let xOffsetCalc = -offsetX;
        const floatingBottom = rects.reference.y + rects.floating.height + offsetY;
        const floatingLeft = rects.reference.x + rects.reference.width - (rects.floating.width + offsetX);
        if (floatingBottom > docRect.bottom) {
            yOffsetCalc -= docRect.bottom - floatingBottom;
        }
        if (floatingLeft < 5) {
            xOffsetCalc += 5 - floatingLeft;
        }
        return {
            mainAxis: yOffsetCalc,
            crossAxis: xOffsetCalc,
        };
    };

    const middleware: Middleware[] = [offset(offsetCallback as any)];

    const updatePosition = async () => {
        const referenceEl = props.anchorRef?.current;
        if (!referenceEl || !floatingEl) return;
        const pos = await computePosition(referenceEl, floatingEl, {
            placement: "top-end",
            middleware,
        });
        floatingStyleAtom._set(`position:absolute;left:${pos.x}px;top:${pos.y}px`);
    };

    const registerFloating = (el: HTMLElement) => {
        floatingEl = el;
        requestAnimationFrame(() => {
            const referenceEl = props.anchorRef?.current;
            if (referenceEl instanceof Element && floatingEl instanceof Element) {
                cleanupAutoUpdate?.();
                cleanupAutoUpdate = autoUpdate(referenceEl, floatingEl, updatePosition);
            }
        });
    };

    onCleanup(() => {
        cleanupAutoUpdate?.();
    });

    // When closed, reset search state
    createEffect(() => {
        if (!isOpenAtom()) {
            searchAtom._set("");
            indexAtom._set(0);
            numResultsAtom._set(0);
        }
    });

    // When search changes, reset index/count and call onSearch
    createEffect(() => {
        const search = searchAtom();
        indexAtom._set(0);
        numResultsAtom._set(0);
        props.onSearch?.(search);
    });

    const onPrevWrapper = () => {
        if (props.onPrev) {
            props.onPrev();
        } else {
            const idx = indexAtom();
            const num = numResultsAtom();
            indexAtom._set((idx - 1 + num) % num);
        }
    };

    const onNextWrapper = () => {
        if (props.onNext) {
            props.onNext();
        } else {
            const idx = indexAtom();
            const num = numResultsAtom();
            indexAtom._set((idx + 1) % num);
        }
    };

    const onKeyDown = (e: KeyboardEvent) => {
        if (e.key === "Enter") {
            if (e.shiftKey) {
                onPrevWrapper();
            } else {
                onNextWrapper();
            }
            e.preventDefault();
        }
    };

    const prevDecl: IconButtonDecl = {
        elemtype: "iconbutton",
        icon: "chevron-up",
        title: "Previous Result (Shift+Enter)",
        get disabled() { return numResultsAtom() === 0; },
        click: onPrevWrapper,
    };

    const nextDecl: IconButtonDecl = {
        elemtype: "iconbutton",
        icon: "chevron-down",
        title: "Next Result (Enter)",
        get disabled() { return numResultsAtom() === 0; },
        click: onNextWrapper,
    };

    const closeDecl: IconButtonDecl = {
        elemtype: "iconbutton",
        icon: "xmark-large",
        title: "Close (Esc)",
        click: () => isOpenAtom._set(false),
    };

    const regexDecl = createToggleButtonDecl(regexAtom, "custom@regex", "Regular Expression");
    const wholeWordDecl = createToggleButtonDecl(wholeWordAtom, "custom@whole-word", "Whole Word");
    const caseSensitiveDecl = createToggleButtonDecl(caseSensitiveAtom, "custom@case-sensitive", "Case Sensitive");

    return (
        <Show when={isOpenAtom()}>
            <Portal>
                <div class="search-container" style={floatingStyleAtom()} ref={registerFloating}>
                    <Input
                        placeholder="Search"
                        value={searchAtom()}
                        onChange={(v) => searchAtom._set(v)}
                        onKeyDown={onKeyDown}
                        autoFocus
                    />
                    <div
                        class={clsx("search-results", { hidden: numResultsAtom() === 0 })}
                        aria-live="polite"
                        aria-label="Search Results"
                    >
                        {indexAtom() + 1}/{numResultsAtom()}
                    </div>

                    <Show when={caseSensitiveDecl || wholeWordDecl || regexDecl}>
                        <div class="additional-buttons">
                            <Show when={caseSensitiveDecl}>
                                <ToggleIconButton decl={caseSensitiveDecl} />
                            </Show>
                            <Show when={wholeWordDecl}>
                                <ToggleIconButton decl={wholeWordDecl} />
                            </Show>
                            <Show when={regexDecl}>
                                <ToggleIconButton decl={regexDecl} />
                            </Show>
                        </div>
                    </Show>

                    <div class="right-buttons">
                        <IconButton decl={prevDecl} />
                        <IconButton decl={nextDecl} />
                        <IconButton decl={closeDecl} />
                    </div>
                </div>
            </Portal>
        </Show>
    );
};

export const Search = SearchComponent;

type SearchOptions = {
    anchorRef?: { current: HTMLElement | null };
    viewModel?: ViewModel;
    regex?: boolean;
    caseSensitive?: boolean;
    wholeWord?: boolean;
};

export function useSearch(options?: SearchOptions): SearchProps {
    const searchAtoms: SearchAtoms = {
        searchValue: createSignalAtom(""),
        resultsIndex: createSignalAtom(0),
        resultsCount: createSignalAtom(0),
        isOpen: createSignalAtom(false),
        regex: options?.regex !== undefined ? createSignalAtom(options.regex) : undefined,
        caseSensitive: options?.caseSensitive !== undefined ? createSignalAtom(options.caseSensitive) : undefined,
        wholeWord: options?.wholeWord !== undefined ? createSignalAtom(options.wholeWord) : undefined,
    };

    const anchorRef = options?.anchorRef ?? { current: null };

    if (options?.viewModel) {
        options.viewModel.searchAtoms = searchAtoms;
    }

    return { ...searchAtoms, anchorRef };
}

const createToggleButtonDecl = (
    atom: SignalAtom<boolean> | undefined,
    icon: string,
    title: string
): ToggleIconButtonDecl | null =>
    atom
        ? {
              elemtype: "toggleiconbutton",
              icon,
              title,
              active: atom,
          }
        : null;

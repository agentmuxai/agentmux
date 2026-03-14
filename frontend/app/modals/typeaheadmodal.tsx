// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

import { Input, InputGroup, InputRightElement } from "@/app/element/input";
import { makeIconClass } from "@/util/util";
import clsx from "clsx";
import { createEffect, createSignal, onMount, type JSX } from "solid-js";
import { Portal } from "solid-js/web";

import "./typeaheadmodal.scss";

interface SuggestionsProps {
    suggestions?: SuggestionsType[];
    onSelect?: (_: string) => void;
    selectIndex: number;
    ref?: (el: HTMLDivElement) => void;
}

const Suggestions = (props: SuggestionsProps) => {
    const renderIcon = (icon: string | JSX.Element, color: string) => {
        if (typeof icon === "string") {
            return <i class={makeIconClass(icon, false)} style={{ color: color }}></i>;
        }
        return icon;
    };

    const renderItem = (item: SuggestionBaseItem | SuggestionConnectionItem, index: number) => (
        <div
            onClick={() => {
                if ("onSelect" in item && item.onSelect) {
                    item.onSelect(item.value);
                } else {
                    props.onSelect(item.value);
                }
            }}
            class={clsx("suggestion-item", { selected: props.selectIndex === index })}
        >
            <div class="typeahead-item-name ellipsis">
                {item.icon &&
                    renderIcon(item.icon, "iconColor" in item && item.iconColor ? item.iconColor : "inherit")}
                {item.label}
            </div>
            {"current" in item && item.current && (
                <i class={clsx(makeIconClass("check", false), "typeahead-current-checkbox")} />
            )}
        </div>
    );

    let fullIndex = -1;
    return (
        <div ref={props.ref} class="suggestions">
            {props.suggestions.map((item, index) => {
                if ("headerText" in item) {
                    return (
                        <div>
                            {item.headerText && <div class="suggestion-header">{item.headerText}</div>}
                            {item.items.map((subItem) => {
                                fullIndex += 1;
                                return renderItem(subItem, fullIndex);
                            })}
                        </div>
                    );
                }
                fullIndex += 1;
                return renderItem(item as SuggestionBaseItem, fullIndex);
            })}
        </div>
    );
};

interface TypeAheadModalProps {
    anchorRef: { current: HTMLElement };
    blockRef?: { current: HTMLDivElement };
    suggestions?: SuggestionsType[];
    label?: string;
    class?: string;
    value?: string;
    onChange?: (_: string) => void;
    onSelect?: (_: string) => void;
    onClickBackdrop?: () => void;
    onKeyDown?: (_) => void;
    giveFocusRef?: { current: () => boolean };
    autoFocus?: boolean;
    selectIndex?: number;
}

const TypeAheadModal = (props: TypeAheadModalProps) => {
    const [width, setWidth] = createSignal(0);
    const [height, setHeight] = createSignal(0);
    let modalRef!: HTMLDivElement;
    let inputRef!: HTMLInputElement;
    let inputGroupRef!: HTMLDivElement;
    let suggestionsWrapperRef!: HTMLDivElement;
    let suggestionsRef!: HTMLDivElement;

    // Observe blockRef for dimension changes
    onMount(() => {
        if (!props.blockRef?.current) return;
        const ro = new ResizeObserver((entries) => {
            for (const entry of entries) {
                setWidth(entry.contentRect.width);
                setHeight(entry.contentRect.height);
            }
        });
        ro.observe(props.blockRef.current);
        return () => ro.disconnect();
    });

    createEffect(() => {
        const h = height();
        if (!modalRef || !inputGroupRef || !suggestionsRef || !suggestionsWrapperRef) return;

        const modalStyles = window.getComputedStyle(modalRef);
        const paddingTop = parseFloat(modalStyles.paddingTop) || 0;
        const paddingBottom = parseFloat(modalStyles.paddingBottom) || 0;
        const borderTop = parseFloat(modalStyles.borderTopWidth) || 0;
        const borderBottom = parseFloat(modalStyles.borderBottomWidth) || 0;
        const modalPadding = paddingTop + paddingBottom;
        const modalBorder = borderTop + borderBottom;

        const suggestionsWrapperStyles = window.getComputedStyle(suggestionsWrapperRef);
        const suggestionsWrapperMarginTop = parseFloat(suggestionsWrapperStyles.marginTop) || 0;

        const inputHeight = inputGroupRef.getBoundingClientRect().height;
        let suggestionsTotalHeight = 0;

        const suggestionItems = suggestionsRef.children;
        for (let i = 0; i < suggestionItems.length; i++) {
            suggestionsTotalHeight += suggestionItems[i].getBoundingClientRect().height;
        }

        const totalHeight =
            modalPadding + modalBorder + inputHeight + suggestionsTotalHeight + suggestionsWrapperMarginTop;
        const maxHeight = h * 0.8;
        const computedHeight = totalHeight > maxHeight ? maxHeight : totalHeight;

        modalRef.style.height = `${computedHeight}px`;
        suggestionsWrapperRef.style.height = `${computedHeight - inputHeight - modalPadding - modalBorder - suggestionsWrapperMarginTop}px`;
    });

    createEffect(() => {
        const w = width();
        if (!props.blockRef?.current || !modalRef) return;

        const blockRect = props.blockRef.current.getBoundingClientRect();
        const anchorRect = props.anchorRef.current.getBoundingClientRect();

        const minGap = 20;
        const availableWidth = blockRect.width - minGap * 2;
        let modalWidth = 300;

        if (modalWidth > availableWidth) {
            modalWidth = availableWidth;
        }

        let leftPosition = anchorRect.left - blockRect.left;
        const modalRightEdge = leftPosition + modalWidth;
        const blockRightEdge = blockRect.width - (minGap - 4);

        if (modalRightEdge > blockRightEdge) {
            leftPosition -= modalRightEdge - blockRightEdge;
        }

        if (leftPosition < minGap) {
            leftPosition = minGap;
        }

        modalRef.style.width = `${modalWidth}px`;
        modalRef.style.left = `${leftPosition}px`;
    });

    onMount(() => {
        if (props.giveFocusRef) {
            props.giveFocusRef.current = () => {
                inputRef?.focus();
                return true;
            };
        }

        if (props.anchorRef.current && modalRef) {
            const parentElement = props.anchorRef.current.closest(".block-frame-default-header");
            modalRef.style.top = `${parentElement?.getBoundingClientRect().height}px`;
        }
    });

    const renderBackdrop = (onClick: () => void) => <div class="type-ahead-modal-backdrop" onClick={onClick}></div>;

    const handleKeyDown = (e) => {
        props.onKeyDown && props.onKeyDown(e);
    };

    const handleChange = (value) => {
        props.onChange && props.onChange(value);
    };

    const handleSelect = (value) => {
        props.onSelect && props.onSelect(value);
    };

    if (props.blockRef && props.blockRef.current == null) {
        return null;
    }

    return (
        <Portal mount={props.blockRef.current}>
            <div class="type-ahead-modal-wrapper" onKeyDown={handleKeyDown}>
                {renderBackdrop(props.onClickBackdrop)}
                <div
                    ref={modalRef}
                    class={clsx("type-ahead-modal", props.class, { "has-suggestions": props.suggestions?.length > 0 })}
                >
                    <InputGroup ref={(el) => { inputGroupRef = el; }}>
                        <Input
                            ref={(el) => { inputRef = el; }}
                            onChange={handleChange}
                            value={props.value}
                            autoFocus={props.autoFocus}
                            placeholder={props.label}
                        />
                        <InputRightElement>
                            <i class="fa-regular fa-magnifying-glass"></i>
                        </InputRightElement>
                    </InputGroup>
                    <div
                        ref={suggestionsWrapperRef}
                        class="suggestions-wrapper"
                        style={{
                            "margin-top": props.suggestions?.length > 0 ? "8px" : "0",
                            "overflow-y": "auto",
                        }}
                    >
                        {props.suggestions?.length > 0 && (
                            <Suggestions
                                ref={(el) => { suggestionsRef = el; }}
                                suggestions={props.suggestions}
                                onSelect={handleSelect}
                                selectIndex={props.selectIndex}
                            />
                        )}
                    </div>
                </div>
            </div>
        </Portal>
    );
};

export { TypeAheadModal };

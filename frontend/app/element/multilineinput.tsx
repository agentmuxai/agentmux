// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import clsx from "clsx";
import { createSignal, JSX, onMount, onCleanup, createEffect } from "solid-js";

import "./multilineinput.scss";

interface MultiLineInputProps {
    value?: string;
    className?: string;
    onChange?: (e: InputEvent & { target: HTMLTextAreaElement }) => void;
    onKeyDown?: (e: KeyboardEvent) => void;
    onFocus?: () => void;
    onBlur?: () => void;
    placeholder?: string;
    defaultValue?: string;
    maxLength?: number;
    autoFocus?: boolean;
    disabled?: boolean;
    rows?: number;
    maxRows?: number;
    manageFocus?: (isFocused: boolean) => void;
    ref?: HTMLTextAreaElement | ((el: HTMLTextAreaElement) => void);
}

const MultiLineInput = (props: MultiLineInputProps): JSX.Element => {
    const defaultValue = props.defaultValue ?? "";
    const rows = props.rows ?? 1;
    const maxRows = props.maxRows ?? 5;

    const [internalValue, setInternalValue] = createSignal(defaultValue);
    const [lineHeight, setLineHeight] = createSignal(24); // Default line height fallback of 24px
    const [paddingTop, setPaddingTop] = createSignal(0);
    const [paddingBottom, setPaddingBottom] = createSignal(0);

    let textareaRef!: HTMLTextAreaElement;

    // Function to count the number of lines in the textarea value
    const countLines = (text: string) => {
        return text.split("\n").length;
    };

    const adjustTextareaHeight = () => {
        if (textareaRef) {
            textareaRef.style.height = "auto"; // Reset height to auto first

            const maxHeight = maxRows * lineHeight() + paddingTop() + paddingBottom();
            const currentLines = countLines(textareaRef.value);
            const newHeight = Math.min(textareaRef.scrollHeight, maxHeight);

            const calculatedHeight =
                currentLines <= maxRows
                    ? `${lineHeight() * currentLines + paddingTop() + paddingBottom()}px`
                    : `${newHeight}px`;

            textareaRef.style.height = calculatedHeight;
        }
    };

    const handleInputChange = (e: InputEvent & { target: HTMLTextAreaElement }) => {
        setInternalValue((e.target as HTMLTextAreaElement).value);
        props.onChange?.(e);
        adjustTextareaHeight();
    };

    const handleFocus = () => {
        props.manageFocus?.(true);
        props.onFocus?.();
    };

    const handleBlur = () => {
        props.manageFocus?.(false);
        props.onBlur?.();
    };

    onMount(() => {
        if (textareaRef) {
            const computedStyle = window.getComputedStyle(textareaRef);
            const detectedLineHeight = parseFloat(computedStyle.lineHeight);
            const detectedPaddingTop = parseFloat(computedStyle.paddingTop);
            const detectedPaddingBottom = parseFloat(computedStyle.paddingBottom);

            setLineHeight(detectedLineHeight);
            setPaddingTop(detectedPaddingTop);
            setPaddingBottom(detectedPaddingBottom);
        }
    });

    createEffect(() => {
        // Reactive dependency on these signals
        const _lh = lineHeight();
        const _pt = paddingTop();
        const _pb = paddingBottom();
        const _v = props.value;
        adjustTextareaHeight();
    });

    const inputValue = () => props.value ?? internalValue();

    const overflowY = () => {
        if (!textareaRef) return "hidden";
        return textareaRef.scrollHeight > maxRows * lineHeight() + paddingTop() + paddingBottom()
            ? "auto"
            : "hidden";
    };

    return (
        <textarea
            class={clsx("multiline-input", props.className)}
            ref={(el) => {
                textareaRef = el;
                if (typeof props.ref === "function") props.ref(el);
            }}
            value={inputValue()}
            onInput={handleInputChange as any}
            onKeyDown={props.onKeyDown}
            onFocus={handleFocus}
            onBlur={handleBlur}
            placeholder={props.placeholder}
            maxLength={props.maxLength}
            autofocus={props.autoFocus}
            disabled={props.disabled}
            rows={rows}
            style={{ "overflow-y": overflowY() }}
        />
    );
};

export { MultiLineInput };
export type { MultiLineInputProps };

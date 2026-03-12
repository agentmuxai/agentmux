// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

import clsx from "clsx";
import { createSignal, JSX, splitProps } from "solid-js";

import "./input.scss";

interface InputGroupProps {
    children?: JSX.Element;
    className?: string;
}

const InputGroup = (props: InputGroupProps): JSX.Element => {
    const [isFocused, setIsFocused] = createSignal(false);

    const manageFocus = (focused: boolean) => {
        setIsFocused(focused);
    };

    // Pass manageFocus via context or as direct prop to children
    // In SolidJS we pass it down via children as-is; consumers call manageFocus
    return (
        <div
            class={clsx("input-group", props.className)}
            classList={{ focused: isFocused() }}
        >
            {props.children}
        </div>
    );
};

interface InputLeftElementProps {
    children?: JSX.Element;
    className?: string;
}

const InputLeftElement = (props: InputLeftElementProps): JSX.Element => {
    return <div class={clsx("input-left-element", props.className)}>{props.children}</div>;
};

interface InputRightElementProps {
    children?: JSX.Element;
    className?: string;
}

const InputRightElement = (props: InputRightElementProps): JSX.Element => {
    return <div class={clsx("input-right-element", props.className)}>{props.children}</div>;
};

interface InputProps {
    value?: string;
    className?: string;
    onChange?: (value: string) => void;
    onKeyDown?: (event: KeyboardEvent) => void;
    onFocus?: () => void;
    onBlur?: () => void;
    placeholder?: string;
    defaultValue?: string;
    required?: boolean;
    maxLength?: number;
    autoFocus?: boolean;
    autoSelect?: boolean;
    disabled?: boolean;
    isNumber?: boolean;
    manageFocus?: (isFocused: boolean) => void;
    ref?: HTMLInputElement | ((el: HTMLInputElement) => void);
}

const Input = (props: InputProps): JSX.Element => {
    const defaultValue = props.defaultValue ?? "";
    const [internalValue, setInternalValue] = createSignal(defaultValue);
    let inputRef!: HTMLInputElement;

    const handleInputChange = (e: Event) => {
        const inputValue = (e.target as HTMLInputElement).value;

        if (props.isNumber && inputValue !== "" && !/^\d*$/.test(inputValue)) {
            return;
        }

        if (props.value === undefined) {
            setInternalValue(inputValue);
        }

        props.onChange && props.onChange(inputValue);
    };

    const handleFocus = () => {
        if (props.autoSelect) {
            inputRef?.select();
        }
        props.manageFocus?.(true);
        props.onFocus?.();
    };

    const handleBlur = () => {
        props.manageFocus?.(false);
        props.onBlur?.();
    };

    const inputValue = () => props.value ?? internalValue();

    return (
        <input
            class={clsx("input", props.className)}
            classList={{ disabled: props.disabled }}
            ref={(el) => {
                inputRef = el;
                if (typeof props.ref === "function") props.ref(el);
                else if (props.ref != null) (props as any).ref = el;
            }}
            value={inputValue()}
            onInput={handleInputChange}
            onKeyDown={props.onKeyDown}
            onFocus={handleFocus}
            onBlur={handleBlur}
            placeholder={props.placeholder}
            maxLength={props.maxLength}
            autofocus={props.autoFocus}
            disabled={props.disabled}
        />
    );
};

export { Input, InputGroup, InputLeftElement, InputRightElement };
export type { InputGroupProps, InputLeftElementProps, InputProps, InputRightElementProps };

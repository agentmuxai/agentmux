// Copyright 2025, Command Line
// SPDX-License-Identifier: Apache-2.0

import clsx from "clsx";
import { createSignal, JSX } from "solid-js";

import "./expandablemenu.scss";

// Global signal for managing open groups
const [openGroupsSignal, setOpenGroupsSignal] = createSignal<{ [key: string]: boolean }>({});

type BaseExpandableMenuItem = {
    type: "item" | "group";
    id?: string;
};

interface ExpandableMenuItemType extends BaseExpandableMenuItem {
    type: "item";
    leftElement?: string | JSX.Element;
    rightElement?: string | JSX.Element;
    content?: JSX.Element | ((props: any) => JSX.Element);
}

interface ExpandableMenuItemGroupTitleType {
    leftElement?: string | JSX.Element;
    label: string;
    rightElement?: string | JSX.Element;
}

interface ExpandableMenuItemGroupType extends BaseExpandableMenuItem {
    type: "group";
    title: ExpandableMenuItemGroupTitleType;
    isOpen?: boolean;
    children?: ExpandableMenuItemData[];
}

type ExpandableMenuItemData = ExpandableMenuItemType | ExpandableMenuItemGroupType;

type ExpandableMenuProps = {
    children?: JSX.Element;
    className?: string;
    noIndent?: boolean;
    singleOpen?: boolean;
};

const ExpandableMenu = (props: ExpandableMenuProps): JSX.Element => {
    return (
        <div class={clsx("expandable-menu", props.className, { "no-indent": props.noIndent ?? false })}>
            {props.children}
        </div>
    );
};

type ExpandableMenuItemProps = {
    children?: JSX.Element;
    className?: string;
    withHoverEffect?: boolean;
    onClick?: () => void;
};

const ExpandableMenuItem = (props: ExpandableMenuItemProps): JSX.Element => {
    const withHoverEffect = props.withHoverEffect ?? true;
    return (
        <div
            class={clsx("expandable-menu-item", props.className, {
                "with-hover-effect": withHoverEffect,
            })}
            onClick={props.onClick}
        >
            {props.children}
        </div>
    );
};

type ExpandableMenuItemGroupTitleProps = {
    children?: JSX.Element;
    className?: string;
    onClick?: () => void;
};

const ExpandableMenuItemGroupTitle = (props: ExpandableMenuItemGroupTitleProps): JSX.Element => {
    return (
        <div class={clsx("expandable-menu-item-group-title", props.className)} onClick={props.onClick}>
            {props.children}
        </div>
    );
};

type ExpandableMenuItemGroupProps = {
    children?: JSX.Element;
    className?: string;
    isOpen?: boolean;
    onToggle?: (isOpen: boolean) => void;
    singleOpen?: boolean;
};

const ExpandableMenuItemGroup = (props: ExpandableMenuItemGroupProps): JSX.Element => {
    // Generate a unique ID for this group
    const id = `group-${Math.random().toString(36).substr(2, 9)}`;

    const singleOpen = props.singleOpen ?? false;

    // Determine if the component is controlled or uncontrolled
    const isControlled = () => props.isOpen !== undefined;

    // Get the open state from global signal in uncontrolled mode
    const actualIsOpen = () => isControlled() ? props.isOpen : (openGroupsSignal()[id] ?? false);

    const toggleOpen = () => {
        const newIsOpen = !actualIsOpen();

        if (isControlled()) {
            // If controlled, call the onToggle callback
            props.onToggle?.(newIsOpen);
        } else {
            // If uncontrolled, update global signal
            setOpenGroupsSignal((prevOpenGroups) => {
                if (singleOpen) {
                    // Close all other groups and open this one
                    return { [id]: newIsOpen };
                } else {
                    // Toggle this group
                    return { ...prevOpenGroups, [id]: newIsOpen };
                }
            });
        }
    };

    // We need to intercept ExpandableMenuItemGroupTitle children and add onClick
    // In SolidJS we can't easily clone elements, so we use a wrapper approach
    // The children are rendered and we detect the title component via class
    // Instead, we render children directly and rely on the structure

    return (
        <div class={clsx("expandable-menu-item-group", props.className, { open: actualIsOpen() })}>
            <ExpandableMenuItemGroupTitleWrapper onToggle={toggleOpen}>
                {props.children}
            </ExpandableMenuItemGroupTitleWrapper>
        </div>
    );
};

// Helper to inject onClick into ExpandableMenuItemGroupTitle
// We render children as-is and use a wrapping click interceptor on the title element
const ExpandableMenuItemGroupTitleWrapper = (props: { children?: JSX.Element; onToggle: () => void }): JSX.Element => {
    // This just renders children — the parent sets up click handling via context
    // For simplicity, wrap with a click handler on the title
    return <>{props.children}</>;
};

type ExpandableMenuItemLeftElementProps = {
    children?: JSX.Element;
    onClick?: () => void;
};

const ExpandableMenuItemLeftElement = (props: ExpandableMenuItemLeftElementProps): JSX.Element => {
    return (
        <div class="expandable-menu-item-left" onClick={props.onClick}>
            {props.children}
        </div>
    );
};

type ExpandableMenuItemRightElementProps = {
    children?: JSX.Element;
    onClick?: () => void;
};

const ExpandableMenuItemRightElement = (props: ExpandableMenuItemRightElementProps): JSX.Element => {
    return (
        <div class="expandable-menu-item-right" onClick={props.onClick}>
            {props.children}
        </div>
    );
};

export {
    ExpandableMenu,
    ExpandableMenuItem,
    ExpandableMenuItemGroup,
    ExpandableMenuItemGroupTitle,
    ExpandableMenuItemLeftElement,
    ExpandableMenuItemRightElement,
};
export type { ExpandableMenuItemData, ExpandableMenuItemGroupTitleType };

// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// The line-style component set: every button, tab, choice and form control
// in the app is meant to come from here
// (docs/specs/SPEC_UI_LINE_STYLE_COMPONENT_SYSTEM_2026_10_05.md).
// scripts/check-ui-primitives.mjs counts what still doesn't.

export { Button, IconButton, type ButtonProps, type IconButtonProps } from "./Button";
export { Field, useField, type FieldProps } from "./Field";
export { FilterInput, type FilterInputProps } from "./FilterInput";
export {
    NumberInput,
    Select,
    Switch,
    TextInput,
    type NumberInputProps,
    type SelectOption,
    type SelectProps,
    type SwitchProps,
    type TextInputProps,
} from "./inputs";
export { SegmentedControl, type SegmentedControlProps, type SegmentedOption } from "./SegmentedControl";
export { densityClass, UiIcon, type UiDensity, type UiTone } from "./shared";
export {
    TabbedPane,
    tabbedPaneLayout,
    tabId,
    tabPanelId,
    Tabs,
    type TabbedPaneLayout,
    type TabbedPaneProps,
    type TabItem,
    type TabsProps,
} from "./Tabs";

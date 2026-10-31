// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// The line-style component set: every button, tab, choice and form control
// in the app is meant to come from here
// (docs/specs/SPEC_UI_LINE_STYLE_COMPONENT_SYSTEM_2026_10_05.md).
// scripts/check-ui-primitives.mjs counts what still doesn't.

export { Button, IconButton, type ButtonProps, type ButtonVariant, type IconButtonProps } from "./Button";
export { Card, type CardProps } from "./Card";
export { CardGrid, columnsFor, rowEnd, type CardGridProps } from "./CardGrid";
export { Field, useField, type FieldProps } from "./Field";
export { EmptyState, type EmptyStateProps } from "./EmptyState";
export { FilterInput, type FilterInputProps } from "./FilterInput";
export { Kbd, type KbdProps } from "./Kbd";
export {
    NumberInput,
    Select,
    Switch,
    TextArea,
    TextInput,
    type NumberInputProps,
    type SelectOption,
    type SelectProps,
    type SwitchProps,
    type TextAreaProps,
    type TextInputProps,
} from "./inputs";
export { Notice, type NoticeProps, type NoticeTone } from "./Notice";
export { PaneBody, PaneScroll, type PaneBodyProps, type PaneScrollProps } from "./PaneBody";
export { Pill, type PillProps, type PillTone } from "./Pill";
export { SectionHeader, type SectionHeaderProps } from "./SectionHeader";
export { SegmentedControl, type SegmentedControlProps, type SegmentedOption } from "./SegmentedControl";
export { densityClass, type UiDensity, type UiTone } from "./shared";
export { StatusDot, type StatusDotProps, type StatusTone } from "./StatusDot";
export {
    FilterChip,
    FilterChips,
    Tag,
    TagList,
    TAG_GROUPS,
    type FilterChipProps,
    type FilterChipsProps,
    type TagCorner,
    type TagGroup,
    type TagKind,
    type TagListProps,
    type TagProps,
    type TagSpec,
    type TagTone,
} from "./Tag";
export {
    TabbedPane,
    tabId,
    tabPanelId,
    Tabs,
    type TabbedPaneLayout,
    type TabbedPaneProps,
    type TabItem,
    type TabsProps,
} from "./Tabs";

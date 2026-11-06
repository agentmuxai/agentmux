// Copyright 2025, Command Line Inc.
// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { shortcutFor } from "@/app/keybindings";
import { cn } from "@/util/util";
import { For, JSX, Show } from "solid-js";
import type { QuickTipsState } from "./quicktips-state";

const KeyCap = (props: { children?: JSX.Element }): JSX.Element => {
    return (
        <div class="inline-block px-2 py-1 mx-[1px] font-mono text-[0.85em] text-foreground bg-highlightbg rounded-[3px] border border-border whitespace-nowrap">
            {props.children}
        </div>
    );
};

const IconBox = (props: { children?: JSX.Element; variant?: "accent" | "secondary" }): JSX.Element => {
    const variant = props.variant ?? "accent";
    const colorClasses =
        variant === "secondary"
            ? "text-secondary bg-hover border-border [&_svg]:fill-secondary [&_svg_#arrow1]:fill-primary [&_svg_#arrow2]:fill-primary"
            : "text-accent-400 bg-accent-400/10 border-accent-400/20 [&_svg]:fill-accent-400 [&_svg_#arrow1]:fill-accent-400 [&_svg_#arrow2]:fill-accent-400";

    return (
        <div
            class={cn(
                "text-[20px] min-w-[32px] h-[32px] flex items-center justify-center rounded-md border [&_svg]:h-[16px]",
                colorClasses
            )}
        >
            {props.children}
        </div>
    );
};

/** One shortcut label ("Ctrl+Shift+T", "⇧⌘W") as a key cap. */
const Shortcut = (props: { label: string }): JSX.Element => (
    <Show when={props.label}>
        <div class="flex flex-row items-center">
            <KeyCap>{props.label}</KeyCap>
        </div>
    </Show>
);

const CARD =
    "flex flex-col gap-4 p-5 bg-gradient-to-br from-highlightbg/30 to-transparent hover:from-accent-400/5 rounded-lg border border-border hover:border-accent-400/20 transition-colors duration-300";

const CardTitle = (props: { children: JSX.Element }): JSX.Element => (
    <div class="flex items-center gap-2 text-xl font-bold">
        <div class="w-1 h-6 bg-accent-400 rounded-full" />
        <span class="text-foreground">{props.children}</span>
    </div>
);

/** What the pane says when the filter leaves nothing. */
export const EmptyNotice = (props: { filter?: string }): JSX.Element => {
    return (
        <div class="quicktips-empty p-5 text-secondary">Nothing in Help matches “{props.filter?.trim()}”.</div>
    );
};

/** The pane-header icons, each with the shortcut of its command. */
export const HeaderIconsCard = ({ s }: { s: QuickTipsState }): JSX.Element => {
    return (
        <div class={CARD}>
            <CardTitle>Header Icons</CardTitle>
            <div class="grid grid-cols-[repeat(auto-fit,minmax(min(15rem,100%),1fr))] gap-3">
                <For each={s.headerIcons()}>
                    {(it) => (
                        <div class="flex items-center gap-3 p-2 rounded-md hover:bg-hover transition-colors">
                            <IconBox variant="secondary">
                                <i class={`${/\bfa-(regular|brands)\b/.test(it.icon) ? "" : "fa-solid "}${it.icon} fa-fw`} />
                            </IconBox>
                            <Show when={it.command} fallback={<span class="text-[15px]">{it.label}</span>}>
                                <div class="flex flex-col gap-0.5 flex-1">
                                    <span class="text-[15px]">{it.label}</span>
                                    <Shortcut label={shortcutFor(it.command!)} />
                                </div>
                            </Show>
                        </div>
                    )}
                </For>
            </div>
        </div>
    );
};

/** The keyboard shortcuts, by section. */
export const ShortcutsCard = ({ s }: { s: QuickTipsState }): JSX.Element => {
    return (
        <div class={CARD}>
            <CardTitle>Keyboard Shortcuts</CardTitle>

            {/* Generated from the shortcut table (frontend/app/keybindings/defaults.ts),
                so this list can't disagree with what the keys do. Columns, not a grid:
                the browser fits as many 17rem columns as the pane is wide and balances
                the sections between them, so a short section is followed straight away
                by the next one instead of leaving a gap beside a long one. */}
            <div class="columns-[17rem] gap-x-5">
                <For each={s.shortcutSections()}>
                    {(section) => (
                        <div class="flex flex-col gap-1.5 mb-6 break-inside-avoid">
                            <div class="text-sm text-accent-400 font-semibold uppercase tracking-wide mb-1">
                                {section.category}
                            </div>
                            <For each={section.entries}>
                                {(entry) => (
                                    <div class="flex flex-col gap-0.5 p-2 rounded-md hover:bg-hover transition-colors">
                                        <span class="text-[15px]">{entry.label}</span>
                                        <div class="flex flex-row flex-wrap items-center gap-1">
                                            <For each={entry.keys}>{(k) => <KeyCap>{k}</KeyCap>}</For>
                                        </div>
                                    </div>
                                )}
                            </For>
                        </div>
                    )}
                </For>
            </div>
        </div>
    );
};

/** The mouse gestures, by area. */
export const GesturesCard = ({ s }: { s: QuickTipsState }): JSX.Element => {
    return (
        <div class={CARD}>
            <CardTitle>Mouse and gestures</CardTitle>
            {/* From keybindings/tips.ts, whose rows are anchored to the code
                they describe (tips.test.ts). Same columns as the keyboard list. */}
            <div class="columns-[17rem] gap-x-5">
                <For each={s.gestureSections()}>
                    {(section) => (
                        <div class="quicktips-gestures flex flex-col gap-1.5 mb-6 break-inside-avoid">
                            <div class="text-sm text-accent-400 font-semibold uppercase tracking-wide mb-1">{section.area}</div>
                            <For each={section.entries}>
                                {(entry) => (
                                    <div class="flex flex-col gap-0.5 p-2 rounded-md hover:bg-hover transition-colors">
                                        <span class="text-[15px]">{entry.label}</span>
                                        <div class="flex flex-row flex-wrap items-center gap-1">
                                            <For each={entry.keys}>
                                                {(alt, i) => (
                                                    <>
                                                        <Show when={i() > 0}>
                                                            <span class="text-secondary">or</span>
                                                        </Show>
                                                        <For each={alt}>{(k) => <KeyCap>{k}</KeyCap>}</For>
                                                    </>
                                                )}
                                            </For>
                                            <Show when={entry.where}>
                                                <span class="text-secondary text-[13px]">{entry.where}</span>
                                            </Show>
                                        </div>
                                    </div>
                                )}
                            </For>
                        </div>
                    )}
                </For>
            </div>
        </div>
    );
};

/** The loose tips. */
export const TipsCard = ({ s }: { s: QuickTipsState }): JSX.Element => {
    return (
        <div class={CARD}>
            <CardTitle>More Tips</CardTitle>
            <div class="flex flex-col gap-2">
                <For each={s.tips()}>
                    {(tip) => (
                        <div class="flex items-center gap-3 p-2 rounded-md hover:bg-hover transition-colors">
                            <IconBox variant="secondary">
                                <i class={`fa-solid ${tip.icon} fa-fw`} />
                            </IconBox>
                            <span>
                                <b>{tip.lead}</b> - {tip.text}
                            </span>
                        </div>
                    )}
                </For>
            </div>
        </div>
    );
};

/** The links to more help. */
export const LinksCard = ({ s }: { s: QuickTipsState }): JSX.Element => {
    return (
        <div class={CARD}>
            <CardTitle>Need More Help?</CardTitle>
            <div class="grid grid-cols-[repeat(auto-fit,minmax(min(15rem,100%),1fr))] gap-2">
                <For each={s.links()}>
                    {(link) => (
                        <div class="flex items-center gap-3 p-3 rounded-md bg-panel hover:bg-highlightbg transition-colors">
                            <IconBox variant="secondary">
                                <i class={`${link.icon} fa-fw`} />
                            </IconBox>
                            <a
                                target="_blank"
                                href={link.href}
                                rel="noopener"
                                class="hover:text-accent-400 hover:underline transition-colors font-medium"
                            >
                                {link.label}
                            </a>
                        </div>
                    )}
                </For>
            </div>
        </div>
    );
};

/** The alpha-software notice, shown while nothing is typed. */
export const AlphaNotice = (): JSX.Element => {
    return (
        <div class="flex flex-col gap-4 p-5 bg-gradient-to-br from-amber-900/20 to-transparent rounded-lg border border-amber-500/20">
            <div class="flex items-center gap-2 text-xl font-bold">
                <div class="w-1 h-6 bg-amber-400 rounded-full" />
                <span class="text-foreground">Alpha Software &amp; AI Content</span>
            </div>
            <div class="flex flex-col gap-2 text-secondary text-sm">
                <p>
                    AgentMux is <b>alpha software</b>. Features may be incomplete, unstable, or change between releases.
                    AI agents generate code, text, and other outputs that may be <b>inaccurate, incomplete, or inappropriate</b>.
                    Always review agent outputs before using them.
                </p>
                <p>
                    To report bugs, request features, or flag issues with AI-generated content:
                </p>
                <div class="flex items-center gap-3 p-3 rounded-md bg-panel hover:bg-highlightbg transition-colors mt-1">
                    <IconBox variant="secondary">
                        <i class="fa-brands fa-github fa-fw" />
                    </IconBox>
                    <a
                        target="_blank"
                        href="https://github.com/agentmuxai/agentmux/issues/new"
                        rel="noopener"
                        class="hover:text-accent-400 hover:underline transition-colors font-medium"
                    >
                        Open an Issue on GitHub
                    </a>
                </div>
            </div>
        </div>
    );
};

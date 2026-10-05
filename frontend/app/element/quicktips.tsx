// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { shortcutFor, shortcutHelp } from "@/app/keybindings";
import { cn } from "@/util/util";
import { For, JSX, Show } from "solid-js";

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

const QuickTips = (): JSX.Element => {
    return (
        <div class="flex flex-col w-full gap-6 @container">
            <div class="flex flex-col gap-4 p-5 bg-gradient-to-br from-highlightbg/30 to-transparent hover:from-accent-400/5 rounded-lg border border-border hover:border-accent-400/20 transition-colors duration-300">
                <div class="flex items-center gap-2 text-xl font-bold">
                    <div class="w-1 h-6 bg-accent-400 rounded-full" />
                    <span class="text-foreground">Header Icons</span>
                </div>
                <div class="grid grid-cols-[repeat(auto-fill,minmax(15rem,1fr))] gap-3">
                    <div class="flex items-center gap-3 p-2 rounded-md hover:bg-hover transition-colors">
                        <IconBox variant="secondary">
                            <i class="fa-solid fa-sharp fa-window-maximize fa-fw" />
                        </IconBox>
                        <div class="flex flex-col gap-0.5 flex-1">
                            <span class="text-[15px]">Maximize a pane</span>
                            <Shortcut label={shortcutFor("pane:magnify")} />
                        </div>
                    </div>
                    <div class="flex items-center gap-3 p-2 rounded-md hover:bg-hover transition-colors">
                        <IconBox variant="secondary">
                            <i class="fa-solid fa-sharp fa-laptop fa-fw" />
                        </IconBox>
                        <div class="flex flex-col gap-0.5 flex-1">
                            <span class="text-[15px]">Change connection</span>
                            <Shortcut label={shortcutFor("pane:changeConnection")} />
                        </div>
                    </div>
                    <div class="flex items-center gap-3 p-2 rounded-md hover:bg-hover transition-colors">
                        <IconBox variant="secondary">
                            <i class="fa-solid fa-sharp fa-cog fa-fw" />
                        </IconBox>
                        <span class="text-[15px]">Pane Settings</span>
                    </div>
                    <div class="flex items-center gap-3 p-2 rounded-md hover:bg-hover transition-colors">
                        <IconBox variant="secondary">
                            <i class="fa-solid fa-sharp fa-xmark-large fa-fw" />
                        </IconBox>
                        <div class="flex flex-col gap-0.5 flex-1">
                            <span class="text-[15px]">Close pane</span>
                            <Shortcut label={shortcutFor("pane:close")} />
                        </div>
                    </div>
                </div>
            </div>

            <div class="flex flex-col gap-4 p-5 bg-gradient-to-br from-highlightbg/30 to-transparent hover:from-accent-400/5 rounded-lg border border-border hover:border-accent-400/20 transition-colors duration-300">
                <div class="flex items-center gap-2 text-xl font-bold">
                    <div class="w-1 h-6 bg-accent-400 rounded-full" />
                    <span class="text-foreground">Keyboard Shortcuts</span>
                </div>

                {/* Generated from the shortcut table (frontend/app/keybindings/defaults.ts),
                    so this list can't disagree with what the keys do. Columns, not a grid:
                    the browser fits as many 17rem columns as the pane is wide and balances
                    the sections between them, so a short section is followed straight away
                    by the next one instead of leaving a gap beside a long one. */}
                <div class="columns-[17rem] gap-x-5">
                    <For each={shortcutHelp()}>
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
                    <div class="flex flex-col gap-1.5 mb-6 break-inside-avoid">
                        <div class="text-sm text-accent-400 font-semibold uppercase tracking-wide mb-1">Mouse</div>
                        <div class="flex flex-col gap-0.5 p-2 rounded-md hover:bg-hover transition-colors">
                            <span class="text-[15px]">Resize a single border</span>
                            <KeyCap>Shift + drag</KeyCap>
                        </div>
                    </div>
                </div>
            </div>

            <div class="flex flex-col gap-4 p-5 bg-gradient-to-br from-highlightbg/30 to-transparent hover:from-accent-400/5 rounded-lg border border-border hover:border-accent-400/20 transition-colors duration-300">
                <div class="flex items-center gap-2 text-xl font-bold">
                    <div class="w-1 h-6 bg-accent-400 rounded-full" />
                    <span class="text-foreground">More Tips</span>
                </div>
                <div class="flex flex-col gap-2">
                    <div class="flex items-center gap-3 p-2 rounded-md hover:bg-hover transition-colors">
                        <IconBox variant="secondary">
                            <i class="fa-solid fa-sharp fa-computer-mouse fa-fw" />
                        </IconBox>
                        <span>
                            <b>Tabs</b> - Right click any tab to change backgrounds or rename.
                        </span>
                    </div>
                    <div class="flex items-center gap-3 p-2 rounded-md hover:bg-hover transition-colors">
                        <IconBox variant="secondary">
                            <i class="fa-solid fa-sharp fa-cog fa-fw" />
                        </IconBox>
                        <span>
                            <b>Web View</b> - Click the gear in the web view to set your homepage
                        </span>
                    </div>
                    <div class="flex items-center gap-3 p-2 rounded-md hover:bg-hover transition-colors">
                        <IconBox variant="secondary">
                            <i class="fa-solid fa-sharp fa-cog fa-fw" />
                        </IconBox>
                        <span>
                            <b>Terminal</b> - Click the gear in the terminal to set your terminal theme and font size
                        </span>
                    </div>
                </div>
            </div>

            <div class="flex flex-col gap-4 p-5 bg-gradient-to-br from-highlightbg/30 to-transparent hover:from-accent-400/5 rounded-lg border border-border hover:border-accent-400/20 transition-colors duration-300">
                <div class="flex items-center gap-2 text-xl font-bold">
                    <div class="w-1 h-6 bg-accent-400 rounded-full" />
                    <span class="text-foreground">Need More Help?</span>
                </div>
                <div class="grid grid-cols-[repeat(auto-fill,minmax(15rem,1fr))] gap-2">
                    <div class="flex items-center gap-3 p-3 rounded-md bg-panel hover:bg-highlightbg transition-colors">
                        <IconBox variant="secondary">
                            <i class="fa-brands fa-discord fa-fw" />
                        </IconBox>
                        <a
                            target="_blank"
                            href="https://discord.com/invite/96erama9Ar"
                            rel="noopener"
                            class="hover:text-accent-400 hover:underline transition-colors font-medium"
                        >
                            Join Our Discord
                        </a>
                    </div>
                    <div class="flex items-center gap-3 p-3 rounded-md bg-panel hover:bg-highlightbg transition-colors">
                        <IconBox variant="secondary">
                            <i class="fa-solid fa-sharp fa-sliders fa-fw" />
                        </IconBox>
                        <a
                            target="_blank"
                            href="https://docs.agentmux.ai/config"
                            rel="noopener"
                            class="hover:text-accent-400 hover:underline transition-colors font-medium"
                        >
                            Configuration Options
                        </a>
                    </div>
                    <div class="flex items-center gap-3 p-3 rounded-md bg-panel hover:bg-highlightbg transition-colors">
                        <IconBox variant="secondary">
                            <i class="fa-solid fa-sharp fa-keyboard fa-fw" />
                        </IconBox>
                        <a
                            target="_blank"
                            href="https://docs.agentmux.ai/keybindings"
                            rel="noopener"
                            class="hover:text-accent-400 hover:underline transition-colors font-medium"
                        >
                            All Keybindings
                        </a>
                    </div>
                    <div class="flex items-center gap-3 p-3 rounded-md bg-panel hover:bg-highlightbg transition-colors">
                        <IconBox variant="secondary">
                            <i class="fa-solid fa-sharp fa-book fa-fw" />
                        </IconBox>
                        <a
                            target="_blank"
                            href="https://docs.agentmux.ai"
                            rel="noopener"
                            class="hover:text-accent-400 hover:underline transition-colors font-medium"
                        >
                            Full Documentation
                        </a>
                    </div>
                    <div class="flex items-center gap-3 p-3 rounded-md bg-panel hover:bg-highlightbg transition-colors">
                        <IconBox variant="secondary">
                            <i class="fa-solid fa-sharp fa-bug fa-fw" />
                        </IconBox>
                        <a
                            target="_blank"
                            href="https://github.com/agentmuxai/agentmux/issues/new"
                            rel="noopener"
                            class="hover:text-accent-400 hover:underline transition-colors font-medium"
                        >
                            Report Bugs &amp; Issues
                        </a>
                    </div>
                </div>
            </div>

            <div class="flex flex-col gap-4 p-5 bg-gradient-to-br from-amber-900/20 to-transparent rounded-lg border border-amber-500/20">
                <div class="flex items-center gap-2 text-xl font-bold">
                    <div class="w-1 h-6 bg-amber-400 rounded-full" />
                    <span class="text-foreground">Alpha Software &amp; AI Content</span>
                </div>
                <div class="flex flex-col gap-2 text-secondary text-sm">
                    <p>
                        AgentMux is <b class="text-amber-300">alpha software</b>. Features may be incomplete, unstable, or change between releases.
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
        </div>
    );
};

export { QuickTips };

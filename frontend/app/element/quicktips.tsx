// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

import { MagnifyIcon } from "@/app/element/magnify";
import { PLATFORM, PlatformMacOS } from "@/util/platformutil";
import { cn } from "@/util/util";
import { For, JSX } from "solid-js";

const KeyCap = (props: { children?: JSX.Element }): JSX.Element => {
    return (
        <div class="inline-block px-2 py-1 mx-[1px] font-mono text-[0.85em] text-foreground bg-highlightbg rounded-[3px] border border-gray-700 whitespace-nowrap">
            {props.children}
        </div>
    );
};

const IconBox = (props: { children?: JSX.Element; variant?: "accent" | "secondary" }): JSX.Element => {
    const variant = props.variant ?? "accent";
    const colorClasses =
        variant === "secondary"
            ? "text-secondary bg-white/5 border-white/10 [&_svg]:fill-secondary [&_svg_#arrow1]:fill-primary [&_svg_#arrow2]:fill-primary"
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

const KeyBinding = (props: { keyDecl: string }): JSX.Element => {
    const chordParts = props.keyDecl.split("+");

    const renderChord = (chordPart: string, chordIdx: number): JSX.Element => {
        const parts = chordPart.trim().split(":");
        const elems: JSX.Element[] = [];

        for (const part of parts) {
            if (part === "Cmd") {
                if (PLATFORM === PlatformMacOS) {
                    elems.push(<KeyCap>⌘ Cmd</KeyCap>);
                } else {
                    elems.push(<KeyCap>Alt</KeyCap>);
                }
                continue;
            }
            if (part === "Ctrl") {
                elems.push(<KeyCap>^ Ctrl</KeyCap>);
                continue;
            }
            if (part === "Shift") {
                elems.push(<KeyCap>⇧ Shift</KeyCap>);
                continue;
            }
            if (part === "Arrows") {
                elems.push(<KeyCap>←</KeyCap>);
                elems.push(<KeyCap>→</KeyCap>);
                elems.push(<KeyCap>↑</KeyCap>);
                elems.push(<KeyCap>↓</KeyCap>);
                continue;
            }
            if (part === "Digit") {
                elems.push(<KeyCap>Number (1-9)</KeyCap>);
                continue;
            }
            if (part === "[" || part === "]") {
                elems.push(<KeyCap>{part}</KeyCap>);
                continue;
            }
            elems.push(<KeyCap>{part.toUpperCase()}</KeyCap>);
        }

        return (
            <div class="flex flex-row items-center gap-1">
                {elems}
            </div>
        );
    };

    return (
        <div class="flex flex-row items-center">
            {chordParts.map((chordPart, chordIdx) => (
                <>
                    {renderChord(chordPart, chordIdx)}
                    {chordIdx < chordParts.length - 1 && (
                        <span class="text-secondary mx-1">+</span>
                    )}
                </>
            ))}
        </div>
    );
};

const QuickTips = (): JSX.Element => {
    return (
        <div class="flex flex-col w-full gap-6 @container">
            <div class="flex flex-col gap-4 p-5 bg-gradient-to-br from-highlightbg/30 to-transparent hover:from-accent-400/5 rounded-lg border border-white/10 hover:border-accent-400/20 transition-all duration-300">
                <div class="flex items-center gap-2 text-xl font-bold">
                    <div class="w-1 h-6 bg-accent-400 rounded-full" />
                    <span class="text-foreground">Header Icons</span>
                </div>
                <div class="grid grid-cols-1 @lg:grid-cols-2 gap-3">
                    <div class="flex items-center gap-3 p-2 rounded-md hover:bg-white/5 transition-colors">
                        <IconBox variant="secondary">
                            <MagnifyIcon enabled={false} />
                        </IconBox>
                        <div class="flex flex-col gap-0.5 flex-1">
                            <span class="text-[15px]">Magnify a Block</span>
                            <KeyBinding keyDecl="Cmd:m" />
                        </div>
                    </div>
                    <div class="flex items-center gap-3 p-2 rounded-md hover:bg-white/5 transition-colors">
                        <IconBox variant="secondary">
                            <i class="fa-solid fa-sharp fa-laptop fa-fw" />
                        </IconBox>
                        <div class="flex flex-col gap-0.5 flex-1">
                            <span class="text-[15px]">Connect to a remote server</span>
                            <KeyBinding keyDecl="Cmd:g" />
                        </div>
                    </div>
                    <div class="flex items-center gap-3 p-2 rounded-md hover:bg-white/5 transition-colors">
                        <IconBox variant="secondary">
                            <i class="fa-solid fa-sharp fa-cog fa-fw" />
                        </IconBox>
                        <span class="text-[15px]">Block Settings</span>
                    </div>
                    <div class="flex items-center gap-3 p-2 rounded-md hover:bg-white/5 transition-colors">
                        <IconBox variant="secondary">
                            <i class="fa-solid fa-sharp fa-xmark-large fa-fw" />
                        </IconBox>
                        <div class="flex flex-col gap-0.5 flex-1">
                            <span class="text-[15px]">Close Block</span>
                            <KeyBinding keyDecl="Cmd:w" />
                        </div>
                    </div>
                </div>
            </div>

            <div class="flex flex-col gap-4 p-5 bg-gradient-to-br from-highlightbg/30 to-transparent hover:from-accent-400/5 rounded-lg border border-white/10 hover:border-accent-400/20 transition-all duration-300">
                <div class="flex items-center gap-2 text-xl font-bold">
                    <div class="w-1 h-6 bg-accent-400 rounded-full" />
                    <span class="text-foreground">Important Keybindings</span>
                </div>

                <div class="grid grid-cols-1 @lg:grid-cols-2 gap-x-5 gap-y-6">
                    <div class="flex flex-col gap-1.5">
                        <div class="text-sm text-accent-400 font-semibold uppercase tracking-wide mb-1">
                            Main Keybindings
                        </div>
                        <div class="flex flex-col gap-0.5 p-2 rounded-md hover:bg-white/5 transition-colors">
                            <span class="text-[15px]">New Tab</span>
                            <KeyBinding keyDecl="Cmd:t" />
                        </div>
                        <div class="flex flex-col gap-0.5 p-2 rounded-md hover:bg-white/5 transition-colors">
                            <span class="text-[15px]">New Terminal Block</span>
                            <KeyBinding keyDecl="Cmd:n" />
                        </div>
                        <div class="flex flex-col gap-0.5 p-2 rounded-md hover:bg-white/5 transition-colors">
                            <span class="text-[15px]">Open Wave AI Panel</span>
                            <KeyBinding keyDecl="Cmd:Shift:a" />
                        </div>
                    </div>

                    <div class="flex flex-col gap-1.5">
                        <div class="text-sm text-accent-400 font-semibold uppercase tracking-wide mb-1">
                            Tab Switching ({PLATFORM === PlatformMacOS ? "Cmd" : "Alt"})
                        </div>
                        <div class="flex flex-col gap-0.5 p-2 rounded-md hover:bg-white/5 transition-colors">
                            <span class="text-[15px]">Switch To Nth Tab</span>
                            <KeyBinding keyDecl="Cmd:Digit" />
                        </div>
                        <div class="flex flex-col gap-0.5 p-2 rounded-md hover:bg-white/5 transition-colors">
                            <span class="text-[15px]">Previous Tab</span>
                            <KeyBinding keyDecl="Cmd:[" />
                        </div>
                        <div class="flex flex-col gap-0.5 p-2 rounded-md hover:bg-white/5 transition-colors">
                            <span class="text-[15px]">Next Tab</span>
                            <KeyBinding keyDecl="Cmd:]" />
                        </div>
                    </div>

                    <div class="flex flex-col gap-1.5">
                        <div class="text-sm text-accent-400 font-semibold uppercase tracking-wide mb-1">
                            Block Navigation (Ctrl-Shift)
                        </div>
                        <div class="flex flex-col gap-0.5 p-2 rounded-md hover:bg-white/5 transition-colors">
                            <span class="text-[15px]">Navigate Between Blocks</span>
                            <KeyBinding keyDecl="Ctrl:Shift:Arrows" />
                        </div>
                        <div class="flex flex-col gap-0.5 p-2 rounded-md hover:bg-white/5 transition-colors">
                            <span class="text-[15px]">Focus Nth Block</span>
                            <KeyBinding keyDecl="Ctrl:Shift:Digit" />
                        </div>
                        <div class="flex flex-col gap-0.5 p-2 rounded-md hover:bg-white/5 transition-colors">
                            <span class="text-[15px]">Focus Wave AI</span>
                            <KeyBinding keyDecl="Ctrl:Shift:0" />
                        </div>
                    </div>

                    <div class="flex flex-col gap-1.5">
                        <div class="text-sm text-accent-400 font-semibold uppercase tracking-wide mb-1">
                            Split Blocks
                        </div>
                        <div class="flex flex-col gap-0.5 p-2 rounded-md hover:bg-white/5 transition-colors">
                            <span class="text-[15px]">Split Right</span>
                            <KeyBinding keyDecl="Cmd:d" />
                        </div>
                        <div class="flex flex-col gap-0.5 p-2 rounded-md hover:bg-white/5 transition-colors">
                            <span class="text-[15px]">Split Below</span>
                            <KeyBinding keyDecl="Cmd:Shift:d" />
                        </div>
                        <div class="flex flex-col gap-0.5 p-2 rounded-md hover:bg-white/5 transition-colors">
                            <span class="text-[15px]">Split in Direction</span>
                            <KeyBinding keyDecl="Ctrl:Shift:s + Arrows" />
                        </div>
                    </div>
                </div>
            </div>

            <div class="flex flex-col gap-4 p-5 bg-gradient-to-br from-highlightbg/30 to-transparent hover:from-accent-400/5 rounded-lg border border-white/10 hover:border-accent-400/20 transition-all duration-300">
                <div class="flex items-center gap-2 text-xl font-bold">
                    <div class="w-1 h-6 bg-accent-400 rounded-full" />
                    <span class="text-foreground">wsh commands</span>
                </div>
                <div class="grid grid-cols-1 @md:grid-cols-2 gap-4">
                    <div class="flex flex-col gap-2 p-4 bg-black/20 rounded-lg border border-accent-400/30 hover:border-accent-400/50 transition-colors">
                        <code class="font-mono text-sm">
                            <span class="text-secondary">&gt; </span>
                            <span class="text-accent-400 font-semibold">wsh view</span>
                            <span class="text-muted"> [filename|url]</span>
                        </code>
                        <div class="text-secondary text-sm mt-1">Preview files, directories, or web URLs</div>
                    </div>
                    <div class="flex flex-col gap-2 p-4 bg-black/20 rounded-lg border border-accent-400/30 hover:border-accent-400/50 transition-colors">
                        <code class="font-mono text-sm">
                            <span class="text-secondary">&gt; </span>
                            <span class="text-accent-400 font-semibold">wsh edit</span>
                            <span class="text-muted"> [filename]</span>
                        </code>
                        <div class="text-secondary text-sm mt-1">Edit config and code files</div>
                    </div>
                </div>
            </div>

            <div class="flex flex-col gap-4 p-5 bg-gradient-to-br from-highlightbg/30 to-transparent hover:from-accent-400/5 rounded-lg border border-white/10 hover:border-accent-400/20 transition-all duration-300">
                <div class="flex items-center gap-2 text-xl font-bold">
                    <div class="w-1 h-6 bg-accent-400 rounded-full" />
                    <span class="text-foreground">More Tips</span>
                </div>
                <div class="flex flex-col gap-2">
                    <div class="flex items-center gap-3 p-2 rounded-md hover:bg-white/5 transition-colors">
                        <IconBox variant="secondary">
                            <i class="fa-solid fa-sharp fa-computer-mouse fa-fw" />
                        </IconBox>
                        <span>
                            <b>Tabs</b> - Right click any tab to change backgrounds or rename.
                        </span>
                    </div>
                    <div class="flex items-center gap-3 p-2 rounded-md hover:bg-white/5 transition-colors">
                        <IconBox variant="secondary">
                            <i class="fa-solid fa-sharp fa-cog fa-fw" />
                        </IconBox>
                        <span>
                            <b>Web View</b> - Click the gear in the web view to set your homepage
                        </span>
                    </div>
                    <div class="flex items-center gap-3 p-2 rounded-md hover:bg-white/5 transition-colors">
                        <IconBox variant="secondary">
                            <i class="fa-solid fa-sharp fa-cog fa-fw" />
                        </IconBox>
                        <span>
                            <b>Terminal</b> - Click the gear in the terminal to set your terminal theme and font size
                        </span>
                    </div>
                </div>
            </div>

            <div class="flex flex-col gap-4 p-5 bg-gradient-to-br from-highlightbg/30 to-transparent hover:from-accent-400/5 rounded-lg border border-white/10 hover:border-accent-400/20 transition-all duration-300">
                <div class="flex items-center gap-2 text-xl font-bold">
                    <div class="w-1 h-6 bg-accent-400 rounded-full" />
                    <span class="text-foreground">Need More Help?</span>
                </div>
                <div class="grid grid-cols-1 @sm:grid-cols-2 gap-2">
                    <div class="flex items-center gap-3 p-3 rounded-md bg-black/20 hover:bg-black/30 transition-colors cursor-pointer">
                        <IconBox variant="secondary">
                            <i class="fa-brands fa-discord fa-fw" />
                        </IconBox>
                        <a
                            target="_blank"
                            href="https://discord.gg/XfvZ334gwU"
                            rel="noopener"
                            class="hover:text-accent-400 hover:underline transition-colors font-medium"
                        >
                            Join Our Discord
                        </a>
                    </div>
                    <div class="flex items-center gap-3 p-3 rounded-md bg-black/20 hover:bg-black/30 transition-colors cursor-pointer">
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
                    <div class="flex items-center gap-3 p-3 rounded-md bg-black/20 hover:bg-black/30 transition-colors cursor-pointer">
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
                    <div class="flex items-center gap-3 p-3 rounded-md bg-black/20 hover:bg-black/30 transition-colors cursor-pointer">
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
                </div>
            </div>
        </div>
    );
};

export { KeyBinding, QuickTips };

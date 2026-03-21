// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

import { CopyButton } from "@/app/element/copybutton";
import { writeText as clipboardWriteText } from "@/util/clipboard";
import { ErrorBoundary } from "@/app/element/errorboundary";
import { IconButton } from "@/app/element/iconbutton";
import { cn, useAtomValueSafe } from "@/util/util";
import { createEffect, createMemo, createSignal, JSX, onCleanup, Show } from "solid-js";
import { Streamdown as StreamdownReact } from "streamdown";
// Cast to any so SolidJS JSX doesn't complain about React component type
const Streamdown = StreamdownReact as any;
import { throttle } from "throttle-debounce";

const ShikiTheme = "github-dark-high-contrast";

// Lazy-load shiki to avoid 9 MB in the initial bundle.
let shikiModule: typeof import("shiki/bundle/web") | null = null;
const getShiki = async () => {
    if (!shikiModule) {
        shikiModule = await import("shiki/bundle/web");
    }
    return shikiModule;
};

function extractText(node: JSX.Element): string {
    if (node == null || typeof node === "boolean") return "";
    if (typeof node === "string" || typeof node === "number") return String(node);
    if (Array.isArray(node)) return (node as JSX.Element[]).map(extractText).join("");
    if (typeof node === "object" && (node as any).props) return extractText((node as any).props.children);
    return "";
}

function CodePlain(props: { className?: string; isCodeBlock: boolean; text: string }): JSX.Element {
    const className = props.className ?? "";
    if (props.isCodeBlock) {
        return <code class={cn("font-mono text-[12px]", className)}>{props.text}</code>;
    }

    return (
        <code class={cn("text-secondary font-mono text-[12px] rounded-sm bg-gray-800 px-1.5 py-0.5", className)}>
            {props.text}
        </code>
    );
}

function CodeHighlight(props: { className?: string; lang: string; text: string }): JSX.Element {
    const className = props.className ?? "";
    const [html, setHtml] = createSignal<string>("");
    const [hasError, setHasError] = createSignal(false);
    let codeEl: HTMLElement | undefined;
    let seqRef = 0;

    const highlightCode = async (
        textToHighlight: string,
        language: string,
        disposedRef: { current: boolean },
        seq: number
    ) => {
        try {
            const { codeToHtml } = await getShiki();
            const full = await codeToHtml(textToHighlight, { lang: language, theme: ShikiTheme });
            const start = full.indexOf("<code");
            const open = full.indexOf(">", start);
            const end = full.lastIndexOf("</code>");
            const inner = start !== -1 && open !== -1 && end !== -1 ? full.slice(open + 1, end) : "";
            if (!disposedRef.current && seq === seqRef) {
                setHtml(inner);
                setHasError(false);
            }
        } catch (e) {
            if (!disposedRef.current && seq === seqRef) {
                setHasError(true);
            }
            console.warn(`Shiki highlight failed for ${language}`, e);
        }
    };

    const throttledHighlight = throttle(300, highlightCode, { noLeading: false });

    createEffect(() => {
        const text = props.text;
        const lang = props.lang;
        const disposedRef = { current: false };

        if (!text) {
            setHtml("");
            return;
        }

        seqRef++;
        const currentSeq = seqRef;
        throttledHighlight(text, lang, disposedRef, currentSeq);

        onCleanup(() => {
            disposedRef.current = true;
        });
    });

    return (
        <Show
            when={!hasError()}
            fallback={
                <code ref={codeEl} class={cn("font-mono text-[12px]", className)}>
                    {props.text}
                </code>
            }
        >
            <Show
                when={html() || !props.text}
                fallback={
                    <code ref={codeEl} class={cn("font-mono text-[12px] text-transparent", className)}>
                        {props.text}
                    </code>
                }
            >
                <code
                    ref={codeEl}
                    class={cn("font-mono text-[12px]", className)}
                    innerHTML={html()}
                />
            </Show>
        </Show>
    );
}

export function Code(props: { className?: string; children?: JSX.Element }): JSX.Element {
    const className = props.className ?? "";
    const m = className?.match(/language-([\w+-]+)/i);
    const isCodeBlock = !!m;
    const lang = m?.[1] || "text";
    const text = extractText(props.children);
    const [hasShikiLang, setHasShikiLang] = createSignal(false);

    createEffect(() => {
        if (isCodeBlock) {
            getShiki().then(({ bundledLanguages }) => {
                setHasShikiLang(lang in bundledLanguages);
            });
        }
    });

    return (
        <Show
            when={isCodeBlock && hasShikiLang()}
            fallback={<CodePlain className={className} isCodeBlock={isCodeBlock} text={text} />}
        >
            <CodeHighlight className={className} lang={lang} text={text} />
        </Show>
    );
}

type CodeBlockProps = {
    children?: JSX.Element;
    onClickExecute?: (cmd: string) => void;
    codeBlockMaxWidthAtom?: (() => number) | null;
};

const CodeBlock = (props: CodeBlockProps): JSX.Element => {
    const codeBlockMaxWidth = () => useAtomValueSafe(props.codeBlockMaxWidthAtom);

    const getLanguage = (children: any): string => {
        if (children?.props?.className) {
            const match = children.props.className.match(/language-([\w+-]+)/i);
            if (match) return match[1];
        }
        return "text";
    };

    const handleCopy = async (e: MouseEvent) => {
        const textToCopy = extractText(props.children).replace(/\n$/, "");
        await clipboardWriteText(textToCopy);
    };

    const handleExecute = (e: MouseEvent) => {
        const cmd = extractText(props.children).replace(/\n$/, "");
        if (props.onClickExecute) {
            props.onClickExecute(cmd);
        }
    };

    const language = getLanguage(props.children);

    return (
        <div
            class={cn("rounded-lg overflow-hidden bg-black my-4", codeBlockMaxWidth() ? "max-w-full" : "")}
            style={
                codeBlockMaxWidth()
                    ? { "max-width": `${codeBlockMaxWidth()}px`, "min-width": `${Math.min(400, codeBlockMaxWidth()!)}px` }
                    : undefined
            }
        >
            <div class="flex items-center justify-between pl-3 pr-2 pt-2 pb-1.5">
                <span class="text-[11px] text-white/50">{language}</span>
                <div class="flex items-center gap-2">
                    <CopyButton onClick={handleCopy} title="Copy" />
                    <Show when={props.onClickExecute}>
                        <IconButton
                            decl={{
                                elemtype: "iconbutton",
                                icon: "regular@square-terminal",
                                click: handleExecute,
                            }}
                        />
                    </Show>
                </div>
            </div>
            <pre class="px-4 pb-2 pt-0 overflow-x-auto m-0 text-secondary max-w-full">{props.children}</pre>
        </div>
    );
};

function Collapsible(props: { title?: JSX.Element; children?: JSX.Element; defaultOpen?: boolean }): JSX.Element {
    const [isOpen, setIsOpen] = createSignal(props.defaultOpen ?? false);

    return (
        <div class="my-3">
            <button
                class="flex items-center gap-2 cursor-pointer bg-transparent border-0 p-0 font-medium text-secondary hover:text-primary"
                onClick={() => setIsOpen(!isOpen())}
            >
                <span class="text-[0.65rem] text-primary transition-transform duration-200 inline-block w-3">
                    {isOpen() ? "\u25BC" : "\u25B6"}
                </span>
                <span>{props.title}</span>
            </button>
            <Show when={isOpen()}>
                <div class="mt-2 ml-1 pl-3.5 border-l-2 border-border text-secondary">{props.children}</div>
            </Show>
        </div>
    );
}

interface WaveStreamdownProps {
    text: string;
    parseIncompleteMarkdown?: boolean;
    className?: string;
    onClickExecute?: (cmd: string) => void;
    codeBlockMaxWidthAtom?: (() => number) | null;
}

export const WaveStreamdown = (props: WaveStreamdownProps): JSX.Element => {
    const components = createMemo(() => ({
        code: Code,
        pre: (preProps: any) => (
            <CodeBlock
                children={preProps.children}
                onClickExecute={props.onClickExecute}
                codeBlockMaxWidthAtom={props.codeBlockMaxWidthAtom}
            />
        ),
        p: (pProps: any) => <p {...pProps} class="text-secondary" />,
        h1: (hProps: any) => <h1 {...hProps} class="text-2xl font-bold text-primary mt-6 mb-3" />,
        h2: (hProps: any) => <h2 {...hProps} class="text-xl font-bold text-primary mt-5 mb-2" />,
        h3: (hProps: any) => <h3 {...hProps} class="text-lg font-bold text-primary mt-4 mb-2" />,
        h4: (hProps: any) => <h4 {...hProps} class="text-base font-semibold text-primary mt-3 mb-1" />,
        h5: (hProps: any) => <h5 {...hProps} class="text-sm font-semibold text-primary mt-2 mb-1" />,
        h6: (hProps: any) => <h6 {...hProps} class="text-sm text-primary mt-2 mb-1" />,
        table: (tProps: any) => <table {...tProps} class="w-full border-collapse my-4" />,
        thead: (thProps: any) => <thead {...thProps} class="border-b border-border" />,
        tbody: (tbProps: any) => <tbody {...tbProps} />,
        tr: (trProps: any) => <tr {...trProps} class="border-b border-border/50 last:border-0" />,
        th: (thProps: any) => <th {...thProps} class="text-left font-semibold px-2 py-1.5 text-sm text-primary" />,
        td: (tdProps: any) => <td {...tdProps} class="px-2 py-1.5 text-sm text-secondary" />,
        ul: (ulProps: any) => (
            <ul
                {...ulProps}
                class="list-disc list-outside pl-6 mt-1 mb-2 text-secondary [&_ul]:my-1 [&_ol]:my-1"
            />
        ),
        ol: (olProps: any) => (
            <ol
                {...olProps}
                class="list-decimal list-outside pl-6 mt-1 mb-2 text-secondary [&_ul]:my-1 [&_ol]:my-1"
            />
        ),
        li: (liProps: any) => <li {...liProps} class="text-secondary leading-snug" />,
        blockquote: (bqProps: any) => (
            <blockquote {...bqProps} class="border-l-2 border-border pl-4 my-2 text-secondary italic" />
        ),
        details: (detailsProps: any) => {
            const { children, ...rest } = detailsProps;
            const childArray = Array.isArray(children) ? children : [children];
            const summary = childArray.find((c: any) => c?.props?.node?.tagName === "summary");
            const summaryText = summary?.props?.children || "Details";
            const content = childArray.filter((c: any) => c?.props?.node?.tagName !== "summary");

            return (
                <Collapsible title={summaryText} defaultOpen={rest.open}>
                    {content}
                </Collapsible>
            );
        },
        summary: () => null,
        a: (aProps: any) => <a {...aProps} class="text-primary underline hover:text-primary/80" />,
        strong: (sProps: any) => <strong {...sProps} class="font-semibold text-secondary" />,
        em: (eProps: any) => <em {...eProps} class="italic text-secondary" />,
    }));

    const streamdownFallback = (
        <pre class="error-boundary" style={{ "white-space": "pre-wrap", padding: "8px" }}>
            Failed to render content
        </pre>
    );

    return (
        <ErrorBoundary fallback={streamdownFallback}>
            <Streamdown
                parseIncompleteMarkdown={props.parseIncompleteMarkdown}
                className={cn(
                    "wave-streamdown text-secondary [&>*:first-child]:mt-0 [&>*:first-child>*:first-child]:mt-0 space-y-2",
                    props.className
                )}
                shikiTheme={[ShikiTheme, ShikiTheme]}
                controls={{
                    code: false,
                    table: false,
                    mermaid: true,
                }}
                mermaid={{
                    config: {
                        theme: "dark",
                        darkMode: true,
                    },
                }}
                defaultOrigin="http://localhost"
                components={components()}
            >
                {props.text}
            </Streamdown>
        </ErrorBoundary>
    );
};

// Default export for lazy() consumers
export default WaveStreamdown;

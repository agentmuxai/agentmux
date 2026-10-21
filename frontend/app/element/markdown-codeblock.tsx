// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { CopyButton } from "@/app/element/copybutton";
import { ErrorBoundary } from "@/app/element/errorboundary";
import {
    cachedRuns,
    highlightBody,
    type ColoredRun,
} from "@/app/view/agent/components/shell-highlight/embedded-highlight";
import { writeText as clipboardWriteText } from "@/util/clipboard";
import { createSignal, For, Show, type JSX } from "solid-js";
import { IconButton } from "./iconbutton";
import { Mermaid, MermaidErrorFallback } from "./markdown-mermaid";
import "./shiki-themed.scss";

/** The text of a code element's children (strings, or DOM nodes this renderer builds eagerly). */
function childText(children: unknown): string {
    if (children == null) return "";
    if (Array.isArray(children)) return children.map(childText).join("");
    if (typeof children === "string") return children;
    if (children instanceof Node) return children.textContent ?? "";
    return String(children);
}

/**
 * A fenced block coloured by shiki (rehype-mark-code-lang.ts marked it with its
 * language). Plain text at once, the coloured runs when they arrive: only this
 * block re-renders, never the message. Both themes' colours ride along as CSS
 * variables (shiki-themed.scss), so a theme switch needs no re-highlight.
 * SPEC_MARKDOWN_CODE_ON_SHIKI_AND_REDUNDANT_LIBRARIES_2026_10_21.md §4.
 */
function ShikiCode(props: { className: string; lang: string; text: string; children: any }): JSX.Element {
    const [runs, setRuns] = createSignal<ColoredRun[] | null>(cachedRuns(props.text, props.lang) ?? null);
    if (!runs()) {
        void highlightBody(props.text, props.lang).then((r) => {
            if (r) setRuns(r);
        });
    }
    return (
        <code class={`${props.className} sh-shiki`}>
            <Show when={runs()} fallback={props.children}>
                {(list) => <For each={list()}>{(run) => (run.style ? <span style={run.style}>{run.text}</span> : run.text)}</For>}
            </Show>
        </code>
    );
}

const Code = (props: { className?: string; children: any; "data-highlight"?: string }) => {
    const className = props.className ?? "";
    const children = props.children;
    if (/\blanguage-mermaid\b/.test(className)) {
        const text = Array.isArray(children) ? children.join("") : String(children ?? "");
        // Mermaid renders an SVG diagram in place of the source text, so
        // reading .textContent off the rendered DOM (CodeBlock's copy path)
        // would pick up the diagram's own <text> label nodes instead of the
        // chart source. Carry the original source alongside it so copy can
        // recover it directly.
        return (
            <div data-raw-code={text}>
                <ErrorBoundary fallback={<MermaidErrorFallback chart={text} />}>
                    <Mermaid chart={text} />
                </ErrorBoundary>
            </div>
        );
    }
    const lang = props["data-highlight"];
    if (lang) {
        return <ShikiCode className={className} lang={lang} text={childText(children)} children={children} />;
    }
    return <code class={className}>{children}</code>;
};

type CodeBlockProps = {
    children: any;
    onClickExecute?: (cmd: string) => void;
};

const CodeBlock = ({ children, onClickExecute }: CodeBlockProps) => {
    // `children` here is a real DOM node, not a React-style {props:
    // {children}} tree — this markdown renderer builds elements via
    // solid-js/h/jsx-runtime (hast-util-to-jsx-runtime), which creates
    // actual DOM eagerly rather than a virtual-DOM descriptor. Walking
    // .props.children never matches, so the only reliable way to get the
    // rendered text is to read it back off the DOM via a ref.
    let contentRef: HTMLDivElement | undefined;

    const getTextContent = (): string => {
        const rawCode = contentRef?.querySelector<HTMLElement>("[data-raw-code]");
        const text = rawCode ? (rawCode.dataset.rawCode ?? "") : (contentRef?.textContent ?? "");
        return text.replace(/\n$/, "");
    };

    const handleCopy = async (e: MouseEvent) => {
        await clipboardWriteText(getTextContent());
    };

    const handleExecute = (e: MouseEvent) => {
        if (onClickExecute) {
            onClickExecute(getTextContent());
        }
    };

    return (
        <pre class="codeblock">
            <div class="codeblock-content" ref={contentRef}>
                {children}
            </div>
            <div class="codeblock-actions">
                <CopyButton onClick={handleCopy} title="Copy" />
                {onClickExecute && (
                    <IconButton
                        decl={{
                            elemtype: "iconbutton",
                            icon: "terminal",
                            click: handleExecute,
                        }}
                    />
                )}
            </div>
        </pre>
    );
};

export { Code, CodeBlock };

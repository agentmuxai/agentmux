// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { ErrorBoundary } from "@/app/element/errorboundary";
import { createContentBlockPlugin } from "@/app/element/markdown-contentblock-plugin";
import { transformBlocks, type MarkdownContentBlockType } from "@/app/element/markdown-util";
import { findSafeSplitPoint } from "@/app/element/markdown-incremental";

/**
 * Deterministic work counters for the streaming-render invariants. Not a
 * profiler: these exist so a test can assert the SHAPE of the work (does
 * per-commit parse input stay bounded? is the processor built once?) without
 * measuring time. Reset via `__resetMarkdownRenderStats()` in tests.
 */
export const __markdownRenderStats = {
    /** Render-memo runs — one per committed text change. */
    commits: 0,
    /** Total characters handed to `processor.parse` across all commits. */
    parsedChars: 0,
    /** Times the unified processor + plugin chain has been constructed. */
    processorBuilds: 0,
    /** Frozen-prefix segments rendered to DOM (each exactly once per segment). */
    domSegmentRenders: 0,
};

export function __resetMarkdownRenderStats(): void {
    __markdownRenderStats.commits = 0;
    __markdownRenderStats.parsedChars = 0;
    __markdownRenderStats.processorBuilds = 0;
    __markdownRenderStats.domSegmentRenders = 0;
}
import { ALIGN_CLASS_REGEX, rehypeAlignToClass } from "@/app/element/rehype-align-to-class";
import remarkMermaidToTag from "@/app/element/remark-mermaid-to-tag";
import { TableBlock } from "@/app/element/table-block";
import { boundNumber, useAtomValueSafe, cn } from "@/util/util";
import { markEnd, markStart } from "@/perf";
import clsx from "clsx";
import { toJsxRuntime } from "hast-util-to-jsx-runtime";
import { OverlayScrollbars } from "overlayscrollbars";
import { createEffect, createMemo, createRoot, createSignal, onCleanup, onMount, Show } from "solid-js";
import type { JSX } from "solid-js";
import { Fragment, jsx, jsxs } from "solid-js/h/jsx-runtime";
import { unified } from "unified";
import rehypeHighlight from "rehype-highlight";
import rehypeRaw from "rehype-raw";
import rehypeSanitize, { defaultSchema } from "rehype-sanitize";
import rehypeSlug from "rehype-slug";
import RemarkFlexibleToc, { TocItem } from "remark-flexible-toc";
import remarkGfm from "remark-gfm";
import remarkParse from "remark-parse";
import remarkRehype from "remark-rehype";
import { openLink } from "../store/global";
import { rehypeLinkify } from "./rehype-linkify";
import { Code, CodeBlock } from "./markdown-codeblock";
import { MarkdownImg, MarkdownSource, MuxBlock } from "./markdown-media";
import { Mermaid, MermaidErrorFallback } from "./markdown-mermaid";
import "./markdown.scss";

const Link = ({
    setFocusedHeading,
    props,
}: {
    props: JSX.AnchorHTMLAttributes<HTMLAnchorElement>;
    setFocusedHeading: (href: string) => void;
}) => {
    const onClick = (e: MouseEvent) => {
        e.preventDefault();
        const href = (props as any).href as string;
        if (!href) return;
        if (href.startsWith("#")) {
            setFocusedHeading(href);
        } else {
            openLink(href);
        }
    };
    return (
        <a href={(props as any).href} onClick={onClick}>
            {(props as any).children}
        </a>
    );
};

const Heading = ({ props, hnum }: { props: JSX.HTMLAttributes<HTMLHeadingElement>; hnum: number }) => {
    return (
        <div id={(props as any).id} class={clsx("heading", `is-${hnum}`)}>
            {(props as any).children}
        </div>
    );
};

type MarkdownProps = {
    text?: string;
    textAtom?: (() => string) | (() => Promise<string>);
    showTocAtom?: () => boolean;
    style?: JSX.CSSProperties;
    class?: string;
    contentClass?: string;
    onClickExecute?: (cmd: string) => void;
    resolveOpts?: MarkdownResolveOpts;
    scrollable?: boolean;
    /** When true, skip OverlayScrollbars entirely and let `.content` use its
     *  plain CSS `overflow` — a real native/webkit scrollbar, styled by the
     *  same universal `*::-webkit-scrollbar` rule (app.scss) every other
     *  native scroll surface (CodeMirror, etc.) already uses, instead of
     *  OverlayScrollbars' JS-rendered auto-hide overlay track. No effect
     *  when `scrollable` is false. */
    nativeScrollbar?: boolean;
    rehype?: boolean;
    /** When false, skip the (expensive) syntax-highlighting rehype plugin
     *  while keeping every other plugin (sanitize, etc.). */
    highlight?: boolean;
    /** The text is still growing. Only the trailing open block can still
     *  change, so only it is rendered cheaply (without syntax highlighting);
     *  closed blocks before it are rendered at full quality exactly once.
     *  Flipping this re-renders that trailing block and nothing else. Read
     *  reactively. */
    streaming?: boolean;
    fontSizeOverride?: number;
    fixedFontSizeOverride?: number;
};

/**
 * What a rendered `<waveblock>` depends on, as a comparable string: each
 * block's key, id and content length — exactly what MuxBlock displays. Empty
 * for the overwhelmingly common no-blocks document, so it costs nothing on the
 * streaming fast path.
 */
function blocksSignature(blocks: Map<string, MarkdownContentBlockType>): string {
    if (blocks.size === 0) return "";
    let sig = "";
    for (const [key, b] of blocks) sig += `${key}\u0000${b.id}\u0000${b.content.length}\u0001`;
    return sig;
}

const Markdown = (props: MarkdownProps) => {
    // `text` is read via props.text inside the resolvedText memo so
    // streaming markdown (where the parent passes a continuously
    // growing string) re-renders. Other props are mostly static (atom
    // refs, style, class) — destructure those for terseness without
    // losing reactivity on the one prop that needs it.
    // (codex P1 on PR #786 — the streaming buffer keeps MarkdownBlock
    // mounted across token deltas, but Markdown's own destructuring
    // froze the captured `text` value at first mount.)
    const {
        textAtom,
        showTocAtom,
        style,
        class: className,
        contentClass: contentClassName,
        resolveOpts,
        fontSizeOverride,
        fixedFontSizeOverride,
        scrollable = true,
        nativeScrollbar = false,
        rehype = true,
        onClickExecute,
    } = props;
    const [focusedHeading, setFocusedHeading] = createSignal<string | null>(null);

    let contentsEl!: HTMLDivElement;
    let tocEl!: HTMLDivElement;
    let contentsOs: OverlayScrollbars | null = null;
    let tocOs: OverlayScrollbars | null = null;

    const [idPrefix] = createSignal<string>(crypto.randomUUID());

    // `useAtomValueSafe(textAtom)` must be called INSIDE the memo, not hoisted
    // to a plain outer `const`. `textAtom` (e.g. the editor's `() => liveDoc()`)
    // is a live reactive getter — calling it once at component-setup time
    // (before setup, before any effect has had a chance to seed the real
    // value) froze the memo's only input, so it never recomputed again for
    // the lifetime of this component instance. On a freshly-mounted markdown
    // preview this snapshot was almost always "" (the seed value hasn't
    // landed yet), producing a permanently blank preview — "fixed" only by
    // remounting the component (e.g. toggling Source → Preview), which
    // re-captures a fresh (by-then-correct) snapshot. Reading the accessor
    // here makes the memo properly track its underlying signal.
    const resolvedText = createMemo(() => useAtomValueSafe<string>(textAtom as any) ?? props.text ?? "");
    const showToc = createMemo(() => useAtomValueSafe(showTocAtom) ?? false);

    const transformedOutput = createMemo(() => transformBlocks(resolvedText()));
    const transformedText = createMemo(() => transformedOutput().content);
    const contentBlocksMap = createMemo(() => transformedOutput().blocks);

    createEffect(() => {
        const heading = focusedHeading();
        if (!heading) return;
        // In OverlayScrollbars mode the real scrolling element is its
        // generated viewport, not contentsEl itself; in nativeScrollbar mode
        // contentsEl IS the scrolling element (plain CSS overflow, no
        // OverlayScrollbars restructuring ever happened).
        const viewport = contentsOs ? contentsOs.elements().viewport : contentsEl;
        if (!viewport) return;
        const el = document.getElementById(idPrefix() + heading.slice(1));
        if (el) {
            const headingRect = el.getBoundingClientRect();
            const viewportRect = viewport.getBoundingClientRect();
            viewport.scrollBy({ top: headingRect.top - viewportRect.top });
        }
    });

    const markdownComponents: Record<string, any> = {
        a: (props: any) => <Link props={props} setFocusedHeading={setFocusedHeading} />,
        p: (props: any) => <div class="paragraph" {...props} />,
        h1: (props: any) => <Heading props={props} hnum={1} />,
        h2: (props: any) => <Heading props={props} hnum={2} />,
        h3: (props: any) => <Heading props={props} hnum={3} />,
        h4: (props: any) => <Heading props={props} hnum={4} />,
        h5: (props: any) => <Heading props={props} hnum={5} />,
        h6: (props: any) => <Heading props={props} hnum={6} />,
        img: (props: any) => <MarkdownImg props={props} resolveOpts={resolveOpts} />,
        source: (props: any) => <MarkdownSource props={props} resolveOpts={resolveOpts} />,
        code: Code,
        pre: (props: any) => <CodeBlock children={props.children} onClickExecute={onClickExecute} />,
        table: (props: any) => <TableBlock>{props.children}</TableBlock>,
        thead: (props: any) => <thead class="border-b border-border bg-hover">{props.children}</thead>,
        tbody: (props: any) => <tbody>{props.children}</tbody>,
        tr: (props: any) => <tr class="border-b border-border/40 last:border-0">{props.children}</tr>,
        th: (props: any) => {
            // Spread sanitizer-survived attributes (colspan, rowspan,
            // scope) so raw HTML tables retain their structure. Codex
            // P2 on PR #754. Override className so alignment classes
            // from rehypeAlignToClass + the cell's typography classes
            // both apply.
            const { children, className, ...rest } = props;
            return (
                <th
                    {...rest}
                    class={cn(
                        "px-3 py-2 text-left text-xs font-semibold uppercase tracking-wide text-primary",
                        className,
                    )}
                >
                    {children}
                </th>
            );
        },
        td: (props: any) => {
            const { children, className, ...rest } = props;
            return (
                <td
                    {...rest}
                    class={cn("px-3 py-2 text-sm text-secondary", className)}
                >
                    {children}
                </td>
            );
        },
        waveblock: (props: any) => <MuxBlock {...props} blockmap={contentBlocksMap()} />,
        mermaidblock: (props: any) => {
            const getTextContent = (children: any): string => {
                if (typeof children === "string") return children;
                if (Array.isArray(children)) return children.map(getTextContent).join("");
                if (children && typeof children === "object" && children.props?.children)
                    return getTextContent(children.props.children);
                return String(children || "");
            };
            const chartText = getTextContent(props.children);
            return (
                <ErrorBoundary fallback={<MermaidErrorFallback chart={chartText} />}>
                    <Mermaid chart={chartText} />
                </ErrorBoundary>
            );
        },
    };

    // Counts WORK, not milliseconds — deliberately.
    //
    // The regression these guard against is algorithmic (re-parsing the whole
    // message per streaming commit, and rebuilding the plugin chain to do it).
    // A wall-clock assertion for that is flaky on shared CI runners to the
    // point of being theater — the position this repo already took in
    // Discussion #1161, and the reason these are counters instead. Counts are
    // deterministic, machine-independent, and fail for a comprehensible
    // reason. See docs/analysis/ANALYSIS_AGENT_PANE_TYPING_UNDER_LOAD_2026_09_22.md
    // and markdown-render-work.test.tsx, which asserts on them.
    //
    // Cost when nobody is looking: three integer adds per render.
    // eslint-disable-next-line
    const stats = __markdownRenderStats;

    // Stable across commits, deliberately: the processor below closes over
    // these once, and each render run refreshes their CONTENTS rather than
    // rebinding them. That is what lets one processor outlive a commit.
    const tocRef: TocItem[] = [];
    const blocksRef = new Map<string, any>();

    /**
     * Already-parsed prefix of the current message: the part far enough from
     * the end that no further streamed text can change how it parses. Holds
     * the toc entries it produced AND the DOM it rendered to, so growth only
     * ever costs a parse of the NEW bytes and a render of the NEW segment.
     * Null means "nothing frozen — parse and render it whole".
     *
     * `elements` are real DOM nodes (Solid's jsx runtime builds eagerly). Each
     * commit hands `[...elements, tail]` back to Solid, whose array reconcile
     * keeps identical node references in place and only inserts/removes the
     * tail — the DOM half of incremental rendering. Before this, the frozen
     * hast was reused but `toJsxRuntime` rebuilt the WHOLE element tree per
     * commit and swapped it, re-creating every paragraph/heading/code block/
     * table of the message ~11×/s (ANALYSIS_AGENT_PANE_FLUSH_REMOUNT_CHURN_2026_09_23.md §6.1).
     *
     * Each segment is rendered under its own `createRoot`, not inside the
     * render memo: a memo disposes every computation it owned when it re-runs,
     * which would strand the segment's components (code-block highlighting,
     * mermaid, waveblocks) with dead effects while their DOM lived on. Roots
     * are disposed when the prefix is invalidated or the component unmounts.
     */
    let frozen: {
        end: number;
        source: string;
        highlight: boolean;
        /** `blocksSignature` of the content-block map the frozen DOM was rendered with. */
        blocks: string;
        toc: TocItem[];
        elements: JSX.Element[];
        disposers: (() => void)[];
    } | null = null;

    const disposeFrozen = (): void => {
        if (!frozen) return;
        for (const d of frozen.disposers) d();
        frozen = null;
    };
    onCleanup(disposeFrozen);

    // One options object for every hast → DOM render below (whole document,
    // frozen segment, trailing block); the casts are the same ones the single
    // pre-incremental call site carried.
    const jsxRuntimeOptions = {
        jsx: jsx as any,
        jsxs: jsxs as any,
        Fragment: Fragment as any,
        passKeys: false,
        components: markdownComponents as any,
    };
    const hastToElement = (children: any[]): JSX.Element =>
        toJsxRuntime({ type: "root", children } as any, jsxRuntimeOptions) as JSX.Element;

    /** Render one independent hast segment to DOM under its own owner. */
    const renderSegment = (children: any[]): { element: JSX.Element; dispose: () => void } =>
        createRoot((dispose) => {
            stats.domSegmentRenders++;
            try {
                return { element: hastToElement(children), dispose };
            } catch (e) {
                // The root exists the moment createRoot runs its callback; if
                // the render throws, nothing above ever receives `dispose`, so
                // the root's reactive scope would leak. Tear it down here and
                // let the render memo's catch handle the error as before.
                // (ReAgent P2 on PR #3559.)
                dispose();
                throw e;
            }
        });

    /** Solid accepts nested arrays, but flatten so the reconcile sees one flat node list.
     *  (Hand-rolled: `Array#flat(Infinity)` over `JSX.Element` trips TS2589.) */
    const flatNodes = (v: JSX.Element): JSX.Element[] => {
        if (!Array.isArray(v)) return [v];
        const out: JSX.Element[] = [];
        for (const item of v) out.push(...flatNodes(item));
        return out;
    };

    // The unified processor used to be constructed INSIDE the render memo, so
    // the entire plugin chain was rebuilt on every streaming commit — ~11x a
    // second per streaming pane, measured at roughly 4ms of the per-commit
    // cost (docs/analysis/ANALYSIS_AGENT_PANE_TYPING_UNDER_LOAD_2026_09_22.md
    // §6.5). It depends only on `highlight` and the slug prefix, never on the
    // text, so it is memoized on those and reused. This matters more, not
    // less, once per-commit parse cost comes down: at ~1ms of parse, a 4ms
    // rebuild would dominate what it is rebuilding.
    // Resolved ONCE as boolean memos. Callers pass these as getters over
    // their own signal (MarkdownBlock: `streaming={view().streaming}`, where
    // `view()` is a new object every streaming commit). Reading the prop
    // directly inside the processor memo tracked that upstream signal, so the
    // whole plugin chain was rebuilt once per commit even though the boolean
    // never changed — live counters read `processorBuilds == commits` under
    // real streaming while the static-prop test stayed green. A memo compares
    // the resolved boolean, so only a real flip propagates.
    // (ANALYSIS_AGENT_PANE_FLUSH_REMOUNT_CHURN_2026_09_23.md §6.)
    const highlightOn = createMemo<boolean>(() => props.highlight ?? true);
    const streamingOn = createMemo<boolean>(() => props.streaming ?? false);

    const buildProcessor = (withHighlight: boolean) => {
        stats.processorBuilds++;
        const rehypePlugins: any[] = rehype
            ? [
                  rehypeRaw,
                  ...(withHighlight ? [rehypeHighlight] : []),
                  rehypeAlignToClass,
                  rehypeLinkify,
                  () =>
                      rehypeSanitize({
                          ...defaultSchema,
                          attributes: {
                              ...defaultSchema.attributes,
                              span: [
                                  ...(defaultSchema.attributes?.span || []),
                                  ["className", /^hljs-./],
                                  ["srcset"],
                                  ["media"],
                                  ["type"],
                              ],
                              th: [
                                  ...(defaultSchema.attributes?.th || []),
                                  ["className", ALIGN_CLASS_REGEX],
                              ],
                              td: [
                                  ...(defaultSchema.attributes?.td || []),
                                  ["className", ALIGN_CLASS_REGEX],
                              ],
                              waveblock: [["blockkey"]],
                          },
                          tagNames: [
                              ...(defaultSchema.tagNames || []),
                              "span",
                              "waveblock",
                              "picture",
                              "source",
                              "mermaidblock",
                          ],
                      }),
                  () => rehypeSlug({ prefix: idPrefix() }),
              ]
            : [];

        const remarkPlugins: any[] = [
            remarkMermaidToTag,
            remarkGfm,
            [RemarkFlexibleToc, { tocRef }],
            [createContentBlockPlugin, { blocks: blocksRef }],
        ];

        return unified()
            .use(remarkParse)
            .use(remarkPlugins as any)
            .use(remarkRehype as any, { allowDangerousHtml: true })
            .use(rehypePlugins as any);
    };

    /** Renders closed blocks: the document's final quality. */
    const processor = createMemo(() => buildProcessor(highlightOn()));
    /** Built on first need, then kept: it never depends on `highlight`. */
    let plainProcessor: ReturnType<typeof buildProcessor> | undefined;
    /**
     * Renders the trailing open block. While streaming it skips highlighting:
     * that block is re-rendered on every commit, and highlighting a code block
     * that is still growing is the expensive part. Everything else about the
     * two processors is identical, so the tail looks the same apart from
     * colour until it closes or the stream settles.
     *
     * The frozen prefix never uses this. It used to be rendered with whichever
     * processor the current commit had, so each streaming ↔ settled flip —
     * which MarkdownBlock infers from a 90 ms quiet gap, and which real
     * streams cross constantly — invalidated it and re-parsed the WHOLE
     * message (TRACKING_AGENT_PANE_BOUNDED_LIVE_WINDOW_2026_09_23.md §3.4;
     * MarkdownBlock.stream-pauses.test.tsx).
     */
    const tailProcessor = () =>
        highlightOn() && streamingOn() ? (plainProcessor ??= buildProcessor(false)) : processor();

    const renderedMarkdown = createMemo(() => {
        const txt = transformedText();
        stats.commits++;
        markStart("markdown-render");

        // Refresh the contents the memoized processor's plugins read at
        // transform time. `tocRef` must be CLEARED, not replaced — it is now
        // shared with the processor, and carrying entries across runs would
        // reintroduce the duplicate/stale-heading accumulation fixed in
        // issue #789.
        blocksRef.clear();
        for (const [k, v] of contentBlocksMap()) blocksRef.set(k, v);

        // The frozen prefix's cache key is the FINAL highlight setting only —
        // never `streaming`, which affects the open tail alone (see
        // `tailProcessor`). Reading both here keeps this memo tracking both.
        const highlight = highlightOn();
        const tailProc = tailProcessor();
        const blocks = blocksSignature(contentBlocksMap());

        /**
         * Parse ONE independent segment. `tocRef` is cleared per call, not per
         * commit, because a commit may now run this more than once and each
         * segment's headings have to be collected separately before the next
         * call wipes them.
         */
        const runSegment = (src: string, proc: ReturnType<typeof buildProcessor>): { children: any[]; toc: TocItem[] } => {
            tocRef.length = 0;
            stats.parsedChars += src.length;
            const hast: any = proc.runSync(proc.parse(src));
            return { children: hast.children ?? [], toc: tocRef.slice() };
        };

        try {
            // Incremental path: freeze the part of the message that can no
            // longer change meaning and parse only the trailing open block, so
            // each byte is parsed once instead of once per commit. Falls back
            // to a whole-document parse whenever a split cannot be PROVEN safe
            // (see markdown-incremental.ts) — that fallback is just today's
            // behavior, so the worst case is unchanged.
            const splitAt = findSafeSplitPoint(txt);

            let element: JSX.Element;
            let toc: TocItem[];

            if (splitAt <= 0) {
                disposeFrozen();
                // Nothing is provably closed, so all of it is the open tail.
                const whole = runSegment(txt, tailProc);
                // Owned by this memo run, like the tail below: disposed and
                // replaced wholesale on the next commit.
                element = hastToElement(whole.children);
                toc = whole.toc;
            } else {
                // The cache is only valid if this really is the same document
                // growing. A non-append edit (history restore, switching
                // messages) or a `highlight` flip invalidates it — the frozen
                // DOM was produced by a different processor in that case. A
                // `streaming` flip does not: frozen segments are closed, so
                // they are always rendered with the final processor.
                // Content-block data is a third input: an `@@@start … @@@end`
                // block becomes a `<waveblock>` placeholder whose text doesn't
                // change when the block's body does, and MuxBlock reads its
                // `blockmap` once at creation — so a frozen placeholder would
                // keep showing the old block. (Codex P2 on #3559.)
                if (frozen && (frozen.highlight !== highlight || frozen.blocks !== blocks || !txt.startsWith(frozen.source))) {
                    disposeFrozen();
                }
                if (!frozen || splitAt > frozen.end) {
                    const from = frozen ? frozen.end : 0;
                    const seg = runSegment(txt.slice(from, splitAt), processor());
                    const rendered = renderSegment(seg.children);
                    frozen = {
                        end: splitAt,
                        source: txt.slice(0, splitAt),
                        highlight,
                        blocks,
                        toc: frozen ? frozen.toc.concat(seg.toc) : seg.toc,
                        elements: frozen ? frozen.elements.concat(flatNodes(rendered.element)) : flatNodes(rendered.element),
                        disposers: frozen ? frozen.disposers.concat(rendered.dispose) : [rendered.dispose],
                    };
                }
                // Only the trailing open block is parsed AND rendered per
                // commit. Its element is owned by this memo run, so the next
                // commit disposes it and Solid's array reconcile swaps just
                // these trailing nodes, leaving `frozen.elements` untouched.
                const tail = runSegment(txt.slice(splitAt), tailProc);
                element = frozen.elements.concat(flatNodes(hastToElement(tail.children)));
                toc = frozen.toc.concat(tail.toc);
            }

            // `toc` is assembled from per-segment copies above, never the live
            // `tocRef` — that array is cleared on the next `runSegment` call,
            // so handing it out would let a consumer observe it empty later.
            // Assembling per segment is also what preserves issue #789's
            // guarantee that the toc reflects only the current text.
            return { element, toc };
        } catch (e) {
            console.error("Markdown render error:", e);
            // A failed incremental parse must not leave a poisoned cache
            // behind for the next commit to build on.
            disposeFrozen();
            return { element: <pre>{txt}</pre>, toc: [] as TocItem[] };
        } finally {
            markEnd("markdown-render", `len=${txt.length}`);
        }
    });

    // Reactive TOC, derived from the render memo. Re-derives whenever the text
    // changes so streaming markdown keeps the TOC in sync, instead of the old
    // non-reactive array that stayed stuck on the first heading set.
    const tocItems = createMemo<TocItem[]>(() => renderedMarkdown().toc);

    onMount(() => {
        if (scrollable && !nativeScrollbar && contentsEl) {
            contentsOs = OverlayScrollbars(contentsEl, { scrollbars: { autoHide: "leave" } });
            onCleanup(() => contentsOs?.destroy());
        }
    });

    const mergedStyle = createMemo((): JSX.CSSProperties => {
        const s: Record<string, any> = { ...(style ?? {}) };
        if (fontSizeOverride != null) {
            s["--markdown-font-size"] = `${boundNumber(fontSizeOverride, 6, 64)}px`;
        }
        if (fixedFontSizeOverride != null) {
            s["--markdown-fixed-font-size"] = `${boundNumber(fixedFontSizeOverride, 6, 64)}px`;
        }
        return s;
    });

    return (
        <div class={clsx("markdown", className)} style={mergedStyle() as any}>
            <Show
                when={scrollable}
                fallback={
                    <div class={cn("content non-scrollable", contentClassName)}>
                        {renderedMarkdown().element}
                    </div>
                }
            >
                <div class={cn("content", contentClassName)} ref={contentsEl}>
                    {/* OverlayScrollbars (below, onMount) restructures contentsEl's
                        DOM once, moving whatever children exist AT INIT TIME into
                        its generated .os-viewport. On first open, liveDoc/textAtom
                        is often still "" when this initial move happens (the real
                        content lands a beat later via a reactive update), which
                        orphaned Solid's insertion anchor for {renderedMarkdown().element}
                        outside the new viewport — a permanently blank preview until
                        something (e.g. a Source/Preview toggle) fully remounted this
                        component. A single, never-replaced wrapper div is the node
                        OverlayScrollbars moves; Solid's reactive updates always target
                        children of THIS stable node, so they keep landing correctly
                        regardless of OverlayScrollbars' init timing. */}
                    <div class="markdown-content-inner">
                        {renderedMarkdown().element}
                    </div>
                </div>
            </Show>
            <Show when={showToc() && tocItems().length > 0}>
                <div class="toc mt-1" ref={tocEl}>
                    <div class="toc-inner">
                        <h4 class="font-bold">Table of Contents</h4>
                        {tocItems().map((item) => (
                            <a
                                class="toc-item"
                                style={{ "--indent-factor": item.depth } as any}
                                onClick={() => setFocusedHeading(item.href)}
                            >
                                {item.value}
                            </a>
                        ))}
                    </div>
                </div>
            </Show>
            <Show when={showToc() && tocItems().length === 0}>
                <div class="toc mt-1">
                    <div class="toc-inner">
                        <h4 class="font-bold">Table of Contents</h4>
                        <div class="toc-item toc-empty text-secondary" style={{ "--indent-factor": 2 } as any}>
                            No sub-headings found
                        </div>
                    </div>
                </div>
            </Show>
        </div>
    );
};

export { Markdown };

// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { t } from "@/app/i18n/i18n";
import { Button, CloseButton, Field, Notice, SectionHeader, SegmentedControl, SwatchGrid, TextInput } from "@/app/element/ui";
import { setBlockMeta } from "@/app/store/block-meta";
import { createEffect, createMemo, createSignal, on, onCleanup, Show, type Accessor, type JSX } from "solid-js";
import { isLightThemeActive } from "../block/pane-identity";
import { deriveBuiltin, type AppScheme } from "./builtins";
import type { DerivedBackdrop } from "./ink";
import { parseBackdrop, sameBackdrop, type BackdropMeta, type BackdropPolarity } from "./meta";
import { builtinOptions, builtinRecord, colorRecord, contrastReadout, parseHexInput, schemeFor } from "./picker-model";
import { useDialogKeys } from "./picker-state";
import { themeAnsi } from "./ansi";
import { registerPaneScope, scopeVars } from "./scope";
import { useBackdropSuppressed, useResolvedBackdrop } from "./usePaneBackdrop";

import "./backdrop-picker.scss";

export interface BackdropPickerProps {
    blockId: string;
    /** The pane's meta: its `backdrop` record, and the old `bg` a legacy pane may still carry. */
    meta: () => { backdrop?: unknown; bg?: unknown } | undefined;
    onClose: () => void;
}

const polarityOptions = () => [
    { value: "follow" as const, label: t("backdrop.polarity.follow") },
    { value: "dark" as const, label: t("backdrop.polarity.dark") },
    { value: "light" as const, label: t("backdrop.polarity.light") },
];

interface CustomColourFieldProps {
    text: string;
    onText: (text: string) => void;
    invalid: boolean;
    /** The colour the native input shows. */
    shown: string;
    /** The typed text is to be used: Enter, or leaving the field with something in it. */
    onApply: () => void;
    /** A colour chosen in the native input, as `#rrggbb`. */
    onPick: (hex: string) => void;
}

/** A custom colour: the native colour input beside a field to type it, with a note when the text is not a colour. */
function CustomColourField(props: CustomColourFieldProps): JSX.Element {
    return (
        <Field label={t("backdrop.custom.label")} layout="stacked" error={props.invalid ? t("backdrop.custom.invalid") : undefined}>
            <div class="backdrop-picker-custom">
                <input
                    type="color"
                    class="backdrop-picker-color"
                    aria-label={t("backdrop.custom.pick")}
                    value={props.shown}
                    onChange={(e) => props.onPick(e.currentTarget.value.toLowerCase())}
                />
                <TextInput
                    placeholder={t("backdrop.custom.placeholder")}
                    value={props.text}
                    onInput={(e) => props.onText(e.currentTarget.value)}
                    onBlur={() => props.text.trim() !== "" && props.onApply()}
                    onKeyDown={(e) => e.key === "Enter" && props.onApply()}
                />
            </div>
        </Field>
    );
}

/** Dark or light keeps a built-in's side under either app theme; Follow takes the app theme's. */
function PolarityField(props: { value: BackdropPolarity; onChange: (next: BackdropPolarity) => void }): JSX.Element {
    return (
        <Field label={t("backdrop.polarity.label")} layout="stacked" hint={t("backdrop.polarity.hint")}>
            <SegmentedControl ariaLabel={t("backdrop.polarity.label")} options={polarityOptions()} value={props.value} onChange={props.onChange} />
        </Field>
    );
}

/**
 * What the pane's backdrop reads like in numbers. When a target was missed it is a warning notice (an icon and a tinted
 * box, so it does not rest on the warning colour alone, which a backdrop tones to near white or black); nothing without a backdrop.
 */
function ContrastReadout(props: { derived: DerivedBackdrop | undefined; missed: boolean }): JSX.Element {
    return (
        <Show when={props.derived}>
            {(d) => (
                <Show
                    when={props.missed}
                    fallback={
                        <p class="backdrop-picker-readout" aria-live="polite">
                            {contrastReadout(d())}
                        </p>
                    }
                >
                    <Notice tone="warning" icon="triangle-exclamation" class="backdrop-picker-readout is-missed">
                        <span aria-live="polite">{contrastReadout(d())}</span>
                    </Notice>
                </Show>
            )}
        </Show>
    );
}

/**
 * The preview of the swatch being looked at (hovered or focused), on the pane's scope. Returns what a swatch calls
 * with its id, or with nothing when it is left. The swatch is kept as state and not only acted on when its event
 * fires: the pane's scope has to follow suppression and the polarity while the swatch stays focused. High Contrast
 * or forced colours coming on takes the preview off the pane (`restore`), and going off puts it back.
 */
function useSwatchPreview(blockId: string, suppressed: Accessor<boolean>, scheme: Accessor<AppScheme>, restore: () => void): (id: string | undefined) => void {
    const [previewId, setPreviewId] = createSignal<string | undefined>();
    createEffect(
        on(
            [previewId, suppressed, scheme],
            ([id, off, side]) => {
                const derived = id && !off ? deriveBuiltin(id, side) : undefined;
                if (derived) registerPaneScope(blockId, scopeVars(derived, themeAnsi()));
                else restore();
            },
            { defer: true }
        )
    );
    return (id) => void setPreviewId(id);
}

/**
 * The pane's backdrop picker (spec §10): an inline panel over the pane's own body, not a window modal. Sixteen
 * swatches, each a small pane in its own ink; a custom colour; polarity; what the choice reads like in numbers; and
 * Reset. Hovering or focusing a swatch previews it on this pane through the scope and reverts when it leaves;
 * choosing writes the pane's `backdrop` record. Esc reverts and closes.
 */
export function BackdropPicker(props: BackdropPickerProps): JSX.Element {
    const appScheme = () => (isLightThemeActive() ? "light" : "dark");
    // Field-wise equal, so the controls below follow the pane's record only when it really changes: any other block-data
    // update (a title, a cwd, another meta key) must not reset a hex being typed or a polarity chosen before a swatch.
    const record = createMemo(() => parseBackdrop(props.meta()?.backdrop), undefined, { equals: sameBackdrop });
    // The same suppression as the pane's own scope: under High Contrast or forced colours nothing is registered, not even a preview.
    const suppressed = useBackdropSuppressed();
    const resolved = useResolvedBackdrop(props.meta, suppressed);

    const [polarity, setPolarity] = createSignal<BackdropPolarity>(record()?.polarity ?? "follow");
    const [hexText, setHexText] = createSignal(record()?.kind === "color" ? (record()!.hex ?? "") : "");
    const [hexError, setHexError] = createSignal(false);
    // The pane's record changed elsewhere (Reset, an agent, another picker): the controls follow it.
    createEffect(() => {
        const r = record();
        setPolarity(r?.polarity ?? "follow");
        setHexText(r?.kind === "color" ? (r.hex ?? "") : "");
        setHexError(false);
    });

    const scheme = () => schemeFor(polarity(), appScheme());
    const options = createMemo(() => builtinOptions(scheme()));

    const write = (next: BackdropMeta | null) => void setBlockMeta(props.blockId, { backdrop: next } as never);
    /** Put back what the pane really has, ending a preview. */
    const restore = () => registerPaneScope(props.blockId, resolved() ? scopeVars(resolved()!.derived, themeAnsi()) : undefined);
    onCleanup(restore);

    const preview = useSwatchPreview(props.blockId, suppressed, scheme, restore);
    const choose = (id: string) => write(builtinRecord(id, polarity()));
    const applyHex = () => {
        const hex = parseHexInput(hexText());
        setHexError(!hex);
        if (hex) write(colorRecord(hex, polarity()));
    };
    const changePolarity = (next: BackdropPolarity) => {
        setPolarity(next);
        const r = record();
        if (r?.kind === "builtin") write(builtinRecord(r.id!, next));
        else if (r?.kind === "color") write(colorRecord(r.hex!, next));
    };
    const reset = () => write(null);
    const close = () => {
        preview(undefined);
        restore();
        props.onClose();
    };

    const missed = () => (resolved()?.derived.unmet.length ?? 0) > 0 || resolved()?.derived.level === "readable";

    let panel: HTMLDivElement | undefined;
    // The picker is the pane's own overlay: a click or focus inside it must not run the pane's click and focus handlers,
    // which hand focus to the composer or the terminal and would take it from the swatch being used.
    const stop = (e: Event) => e.stopPropagation();
    useDialogKeys(() => panel, close);

    return (
        <div ref={panel} class="backdrop-picker" role="dialog" aria-label={t("backdrop.title")} tabIndex={-1} onClick={stop} onMouseDown={stop} onFocusIn={stop}>
            <div class="backdrop-picker-head">
                <SectionHeader variant="title" level={3} title={t("backdrop.title")} />
                <CloseButton size="header" label={t("backdrop.close")} onClick={close} />
            </div>

            <SwatchGrid
                ariaLabel={t("backdrop.colours")}
                options={options()}
                value={record()?.kind === "builtin" ? record()!.id : undefined}
                columns={4}
                onChange={choose}
                onPreview={preview}
            />

            <Show when={suppressed()}>
                <Notice tone="accent">{t("backdrop.suppressed")}</Notice>
            </Show>

            <CustomColourField
                text={hexText()}
                onText={setHexText}
                invalid={hexError()}
                shown={parseHexInput(hexText()) ?? resolved()?.derived.surfaces.surface ?? "#000000"}
                onApply={applyHex}
                onPick={(hex) => write(colorRecord(hex, polarity()))}
            />
            <Show when={record()?.kind === "color" && resolved()?.derived.nudged}>
                <Notice tone="accent">{t("backdrop.nudged")}</Notice>
            </Show>

            <PolarityField value={polarity()} onChange={changePolarity} />
            <ContrastReadout derived={resolved()?.derived} missed={missed()} />

            <Button variant="row" icon="rotate-left" disabled={!record()} onClick={reset}>
                {t("backdrop.reset")}
            </Button>
        </div>
    );
}

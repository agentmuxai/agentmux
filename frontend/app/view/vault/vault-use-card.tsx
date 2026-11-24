// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The whole card the person types when an agent asks for one no Vault entry holds yet (`Browser` `request_value`,
// kind `card`): number, expiry, security code, name and ZIP in the agent's question, and a box to keep it in the Vault
// (docs/specs/SPEC_VAULT_CARD_ENTRIES_FULL_CARD_DATA_AND_AUTOFILL_2026_11_08.md §5 item 3). What is typed leaves as
// one JSON text in the answer's `value`; srv makes it the card's parts. The agent never sees it.

import { createEffect, createSignal, Show, type JSX } from "solid-js";

import { Field, Pill, Select, TextInput } from "@/app/element/ui";
import { t } from "@/app/i18n/i18n";
import { CARD_BRANDS, cardBrand, cardCvcLength, cardDigits, cardGroup, cardLuhn, expiryYears } from "./vault-card";

const MONTHS = Array.from({ length: 12 }, (_, i) => String(i + 1).padStart(2, "0"));

/** The text the answer carries for what was typed: nothing until there is a number, which the card needs. */
export function cardAnswerText(card: { number: string; month: string; year: string; cvc: string; name: string; zip: string }): string {
    if (!cardDigits(card.number)) return "";
    return JSON.stringify({
        number: card.number,
        exp: card.month && card.year ? `${card.month}/${card.year}` : "",
        cvc: card.cvc,
        name: card.name,
        zip: card.zip,
    });
}

/** The expiry: a month and a year to choose from, side by side. */
function CardExpiry(props: { month: string; year: string; busy: boolean; onMonth: (m: string) => void; onYear: (y: string) => void }): JSX.Element {
    return (
        <Field label={t("vault.useCard.expiry")} layout="stacked">
            <div class="vault-part vault-card-expiry">
                <Select
                    ariaLabel={t("vault.useCard.expiryMonth")}
                    value={props.month}
                    disabled={props.busy}
                    onChange={props.onMonth}
                    options={[{ value: "", label: t("vault.useCard.month") }, ...MONTHS.map((m) => ({ value: m, label: m }))]}
                />
                <Select
                    ariaLabel={t("vault.useCard.expiryYear")}
                    value={props.year}
                    disabled={props.busy}
                    onChange={props.onYear}
                    options={[{ value: "", label: t("vault.useCard.year") }, ...expiryYears().map((y) => ({ value: String(y), label: String(y) }))]}
                />
            </div>
        </Field>
    );
}

/** The card form, in the question. `label` is what the agent called the card (already cut for display), `why` what the
 *  person is told about being asked, `keepAs` the label it would be kept under. */
export function CardTyped(props: {
    label: string;
    why: string;
    busy: boolean;
    /** The answer was sent: what was typed is cleared from the boxes, whether or not the question leaves at once. */
    answered: boolean;
    focus: boolean;
    save: boolean;
    onSave: (on: boolean) => void;
    onText: (text: string) => void;
    onSubmit: () => void;
}): JSX.Element {
    const [number, setNumber] = createSignal("");
    const [month, setMonth] = createSignal("");
    const [year, setYear] = createSignal("");
    const [cvc, setCvc] = createSignal("");
    const [name, setName] = createSignal("");
    const [zip, setZip] = createSignal("");
    const digits = () => cardDigits(number());
    const brand = () => (number() ? cardBrand(number()) : "");
    const length = () => cardCvcLength(brand());
    createEffect(() => {
        if (!props.answered) return;
        for (const set of [setNumber, setMonth, setYear, setCvc, setName, setZip]) set("");
    });
    createEffect(() => {
        props.onText(cardAnswerText({ number: number(), month: month(), year: year(), cvc: cvc(), name: name(), zip: zip() }));
    });
    return (
        <form
            class="vault-use-card"
            onSubmit={(e) => {
                e.preventDefault();
                props.onSubmit();
            }}
        >
            <p class="warden-manager-dim">
                {t("vault.useCard.intro", { why: props.why })}
            </p>
            <Field label={t("vault.useCard.number")} layout="stacked">
                <div class="vault-part vault-card-number">
                    <TextInput
                        inputmode="numeric"
                        autocomplete="off"
                        spellcheck={false}
                        autofocus={props.focus}
                        value={number()}
                        onInput={(e) => setNumber(cardGroup(e.currentTarget.value))}
                        disabled={props.busy}
                    />
                    <Show when={brand()}>
                        <Pill>{CARD_BRANDS[brand()]}</Pill>
                    </Show>
                </div>
                <Show when={digits().length >= 12 && !cardLuhn(digits())}>
                    <div class="vault-card-warn" role="note">
                        {t("vault.useCard.warn")}
                    </div>
                </Show>
            </Field>
            <CardExpiry month={month()} year={year()} busy={props.busy} onMonth={setMonth} onYear={setYear} />
            <Field label={length() === 4 ? t("vault.useCard.cid") : t("vault.useCard.cvc")} layout="stacked">
                <TextInput
                    inputmode="numeric"
                    autocomplete="off"
                    spellcheck={false}
                    maxLength={length()}
                    value={cvc()}
                    onInput={(e) => setCvc(cardDigits(e.currentTarget.value))}
                    disabled={props.busy}
                />
            </Field>
            <Field label={t("vault.useCard.name")} layout="stacked">
                <TextInput autocomplete="off" spellcheck={false} value={name()} onInput={(e) => setName(e.currentTarget.value)} disabled={props.busy} />
            </Field>
            <Field label={t("vault.useCard.zip")} layout="stacked">
                <TextInput autocomplete="off" spellcheck={false} value={zip()} onInput={(e) => setZip(e.currentTarget.value)} disabled={props.busy} />
            </Field>
            <label class="vault-check">
                <input type="checkbox" checked={props.save} onChange={(e) => props.onSave(e.currentTarget.checked)} disabled={props.busy} />
                {t("vault.useCard.saveAs", { label: props.label })}
            </label>
            <input type="submit" hidden />
        </form>
    );
}

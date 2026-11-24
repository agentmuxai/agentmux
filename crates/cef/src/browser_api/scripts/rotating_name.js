// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// What stands in for the value of a hidden form field that is one a page rewrites by itself: an anti-forgery
// token, a nonce, a timestamp, a honeypot. act.rs splices this file into COMMIT_INFO as the value of `rotatingName`,
// so it must stay one function expression; rotating_name.test.mjs runs the same text. The approval of a click that
// submits a form is void when anything the form sends changed while the person read it; a hidden field the page
// rewrites every second would void every approval, though nobody could see it change. The page chooses both a
// field's name and its value, so a name alone is not enough: the value must also look like a token (one run of
// letters, digits and + / = _ . : - , no URL, 6 to 512 characters) and what stands in for it records its kind and
// length, so a field that turns into something else, longer or shorter, or a plain word, still voids the approval.
// A name that points at money, a card, an account or a person's details never counts. Returns "" when the field
// is not one, else the stand-in text for its value.
function (name, value) {
  const n = String(name || "").toLowerCase();
  if (!n || typeof value !== "string") return "";
  if (/pay|card|bank|account|iban|amount|price|total|recipient|payee|wallet|address|email|phone|user|dest|target|redirect|return|next|url|to$|^to_/.test(n)) return "";
  if (!/csrf|xsrf|nonce|token|timestamp|honeypot|captcha|authenticity|^_?(ts|time|hp)$/.test(n)) return "";
  if (!/^[A-Za-z0-9+\/=_.:-]{6,512}$/.test(value) || value.includes("://")) return "";
  const kind = /^[0-9]+$/.test(value) ? "digits" : /^[0-9a-f]+$/i.test(value) ? "hex" : "token";
  return "[rewritten by the page: " + kind + " " + value.length + "]";
}

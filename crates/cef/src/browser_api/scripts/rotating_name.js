// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Whether a hidden form field is one a page rewrites by itself, by its name: an anti-forgery token, a nonce, a
// timestamp, a honeypot. act.rs splices this file into COMMIT_INFO as the value of `rotatingName`, so it must
// stay one function expression; rotating_name.test.mjs runs the same text. The approval of a click that submits
// a form is void when anything the form sends changed while the person read it; a hidden field the page
// rewrites every second would void every approval, though nobody could see it change. Only the value of a
// hidden field with such a name is left out of that comparison (its name still counts, so a field that appears
// or goes does void it). A name that points at money, a card, an account or a person's details never counts, so
// a page can't swap what it pays or who it pays under a "token" name.
function (name) {
  const n = String(name || "").toLowerCase();
  if (!n) return false;
  if (/pay|card|bank|account|iban|amount|price|total|recipient|payee|wallet|address|email|phone|user/.test(n)) return false;
  return /csrf|xsrf|nonce|token|timestamp|honeypot|captcha|authenticity|^_?(ts|time|hp)$/.test(n);
}

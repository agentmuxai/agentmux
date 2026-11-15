function () {
  const out = { tag: this.tagName.toLowerCase() };
  if (this.type) out.type = this.type;
  if (this.isContentEditable) { out.value = this.textContent; out.contentEditable = true; }
  else if ("value" in this && this.tagName !== "BUTTON") out.value = this.value;
  if (this.tagName === "SELECT") out.selected = Array.from(this.selectedOptions).map(o => o.label.trim() || o.value);
  if ("checked" in this && (this.type === "checkbox" || this.type === "radio")) out.checked = this.checked;
  const aria = this.getAttribute("aria-checked");
  if (aria !== null && out.checked === undefined) out.checked = aria === "true";
  if (this.validationMessage) out.validationMessage = this.validationMessage;
  const invalid = this.getAttribute("aria-invalid");
  if ((invalid && invalid !== "false") || (this.validity && !this.validity.valid)) out.invalid = true;
  // A link, a button (which always has a type of its own, "submit" by default, so `tag` plus `type` is all
  // it ever gets above), or anything else none of the above says anything about (a div acting as one): the
  // destination and visible text a reader can use, never a bare tag (and maybe a type) on their own.
  if (Object.keys(out).every((k) => k === "tag" || k === "type")) {
    if (this.tagName === "A" && this.hasAttribute("href")) out.href = this.href;
    const text = (this.getAttribute("aria-label") || this.innerText || this.textContent || "").trim().replace(/\s+/g, " ");
    if (text) out.text = text.slice(0, 200);
  }
  return out;
}

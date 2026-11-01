// Runs in an isolated world of each document in a browser pane, beside
// side_button.js (side_button_cdp.rs adds both). It notices a sign-in being sent
// (SPEC_VAULT_GENERATE_CAPTURE_AND_BROWSER_SIGNINS_2026_10_28.md §5.1) and hands
// the user name and password to the host through the binding only this world
// has, so the page's own scripts can't call it. Only the person's own events
// count (`isTrusted`): a page's `dispatchEvent` or `el.click()` report nothing.
(() => {
  if (window.__agentmuxSignIn) return;
  window.__agentmuxSignIn = true;
  // A submit and the click or Enter that caused it report once.
  const REPEAT_MS = 800;
  // How long after a sign-in the password field is watched for leaving the page.
  const WATCH_MS = 15000;
  let lastAt = -Infinity;

  const report = (o) => {
    if (typeof window.agentmuxSignIn === "function") window.agentmuxSignIn(JSON.stringify(o));
  };
  const isInput = (el) => el instanceof HTMLInputElement;
  const shown = (el) => el.isConnected && el.getClientRects().length > 0 && getComputedStyle(el).visibility !== "hidden";
  const typed = (el) => ["text", "email", "tel", ""].includes(el.type);
  const autocomplete = (el) => (el.getAttribute("autocomplete") || "").toLowerCase();
  const namedLikeUser = (el) =>
    /\b(username|email)\b/.test(autocomplete(el)) || el.type === "email" || /user|login|email|account/i.test(`${el.name} ${el.id}`);

  // The fields that go with `el`: its form's, or for fields outside any form, the page's.
  const groupOf = (el) => (el && (el.form || el.closest("form"))) || document;
  const inputsOf = (group) => [...group.querySelectorAll("input")].filter((i) => isInput(i) && shown(i));

  // The user name: the field marked as one, else the last text or email field before the password.
  const userIn = (inputs, password) => {
    const marked = inputs.find((i) => i !== password && /\b(username|email)\b/.test(autocomplete(i)));
    if (marked) return marked;
    const before = password ? inputs.slice(0, inputs.indexOf(password)) : inputs;
    return before.reverse().find((i) => typed(i));
  };

  const watchLeaving = (password) => {
    const until = performance.now() + WATCH_MS;
    const timer = setInterval(() => {
      if (!shown(password)) {
        clearInterval(timer);
        report({ kind: "left" });
      } else if (performance.now() > until) {
        clearInterval(timer);
      }
    }, 500);
  };

  const sent = (group) => {
    const now = performance.now();
    if (now - lastAt < REPEAT_MS) return;
    const inputs = inputsOf(group);
    const passwords = inputs.filter((i) => i.type === "password");
    // Two or three password fields, or one for a new password, is a sign-up or a password change (§6).
    if (passwords.length > 1 || passwords.some((p) => autocomplete(p).includes("new-password"))) return;
    if (passwords.length === 1) {
      const password = passwords[0];
      if (!password.value) return;
      lastAt = now;
      const user = userIn(inputs, password);
      report({ kind: "signin", username: user ? user.value : "", password: password.value });
      watchLeaving(password);
      return;
    }
    // A step that asks only for the user name (Google, Microsoft): held by srv for the password step.
    const user = inputs.find((i) => typed(i) && namedLikeUser(i) && i.value);
    if (user) {
      lastAt = now;
      report({ kind: "identifier", username: user.value });
    }
  };

  const SUBMIT_WORDS = /sign ?in|log ?in|continue|next|submit/i;
  const isSubmitButton = (el) => {
    // A form's submit button; outside a form, a button named like one.
    if (el instanceof HTMLButtonElement) return (el.type === "submit" && !!el.form) || SUBMIT_WORDS.test(el.textContent || "");
    if (isInput(el)) return el.type === "submit" || el.type === "image";
    return el.getAttribute("role") === "button" && SUBMIT_WORDS.test(el.textContent || "");
  };

  window.addEventListener(
    "submit",
    (e) => {
      if (e.isTrusted && e.target instanceof HTMLFormElement) sent(e.target);
    },
    { capture: true }
  );
  window.addEventListener(
    "click",
    (e) => {
      if (!e.isTrusted || !(e.target instanceof Element)) return;
      const button = e.target.closest("button, input[type=submit], input[type=image], [role=button]");
      if (button && isSubmitButton(button)) sent(groupOf(button));
    },
    { capture: true }
  );
  window.addEventListener(
    "keydown",
    (e) => {
      if (e.isTrusted && e.key === "Enter" && isInput(e.target)) sent(groupOf(e.target));
    },
    { capture: true }
  );
})();

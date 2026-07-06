# MS Store CI Auth — Research Findings
**Date:** 2026-06-30
**Question:** Can msstore CLI authenticate in CI without Entra admin access / using a personal Microsoft account?

---

## Bottom line

**Entra is required — but you don't need the Azure portal.**
Partner Center has a built-in path to create a free Entra tenant without ever touching `portal.azure.com` or `entra.microsoft.com`.

---

## Key confirmed findings

### 1. msstore CLI CI mode requires a service principal — no MSA path

The `msstore reconfigure` command (the CI-mode auth setup) takes only:
```
--tenantId   --sellerId   --clientId   --clientSecret
```
(or `--certificateThumbprint` / `--certificateFilePath` variants)

There is no `--token`, `--apiKey`, or personal-account parameter. The CLI overview page (updated June 2026) explicitly warns:

> "When signing in, don't use your MSA! The Microsoft Store Developer CLI requires you to use your Microsoft Entra ID credentials."

**Confirmed by:** msstore CLI docs, Store Submission API docs, multiple GitHub issues (msstore-cli #4, #25). No credible contradicting source found.

### 2. The 401 on entra.microsoft.com is expected for personal MSA holders

Personal Microsoft accounts (`@outlook.com`, `@hotmail.com`) are used to *register* as Windows developers in Partner Center — but they don't automatically give you admin access to an Entra tenant. The tenant ID `4687dc46-1fd2-45e9-9411-ab7a7ad55df5` exists but you're not its admin, hence the 401.

### 3. The fix: create the Entra tenant FROM Partner Center — no Azure portal needed

Partner Center has its own tenant creation wizard that bypasses the 401:

**Partner Center → Account settings → Tenants → Associate Azure AD → Create a brand new Azure AD tenant**

This creates a free Entra tenant scoped to your Partner Center account. No Azure subscription, no `portal.azure.com` access needed.

After creating the tenant, the app registration can also be done from within Partner Center:

**Partner Center → Account settings → Users → Azure AD applications → Add an Azure AD application**

This generates a Client ID + lets you create a Client Secret without going to Entra directly.

### 4. Required role is Manager, not Global Admin

Some Microsoft docs mention "global admin" — those apply to the CSP Partner Center SDK (cloud reseller API), not the Windows Store Submission API. For msstore CLI the required Partner Center role is **Manager**, which is set when you add the Azure AD application to your account.

### 5. No alternative to Entra for CI

No documented workaround exists. A June 2024 Microsoft identity platform breaking change explicitly tightened this by requiring all app registrations to exist within a directory tenant, eliminating the last "tenantless" MSA path.

---

## Recommended setup path (avoids Azure portal entirely)

1. **Partner Center → Account settings → Tenants → Create new Azure AD tenant**
   - Name it (e.g. `agentmux`) — this gives you a new `MSSTORE_TENANT_ID`

2. **Partner Center → Account settings → Users → Azure AD applications → Add**
   - Create new application (e.g. `AgentMux CI`)
   - Assign **Manager** role
   - Copy the **Client ID** → `MSSTORE_CLIENT_ID`
   - Generate a new key (client secret) → `MSSTORE_CLIENT_SECRET`
   - The Tenant ID is from step 1 → `MSSTORE_TENANT_ID`

3. **Profile/Seller ID** is already known: `15588cde-ee0a-4471-a7e9-d5828d87ada6` → `MSSTORE_SELLER_ID`

4. **Store App ID** (`MSSTORE_APP_ID`): Partner Center → Apps & games → AgentMux → App identity → Store ID (format `9NXXXXXXX`)

---

## Why automated MS Store submission is deferred

After exhausting all available paths (2026-06-30):

| Path | Outcome |
|---|---|
| Partner Center → Tenants → Create new Azure AD tenant | Blocked — requires work/school account at sign-in |
| `entra.microsoft.com` | 401 — tenant `4687dc46-...` exists but Microsoft is the admin, not the account holder |
| Microsoft 365 Developer Program sandbox | Rejected — account doesn't meet activity threshold |
| Azure free account tenant creation | Blocked — Microsoft now requires a **paid Entra P1/P2 license** to create new Workforce tenants. Free accounts, trial subscriptions, and verified Azure free accounts are all explicitly blocked. Error: "Customers must own a paid license to create Microsoft Entra Workforce tenant." Introduced after Microsoft's security team flagged free tenant creation as a fraud vector (2024). |

**Decision:** Defer automated Store submission. The MSIX ships with every GH Release; manual upload via Partner Center is the interim process. Revisit when release cadence justifies an Entra P1 license (~$6/user/month).

The `ms-store` job remains in `release.yml` with `continue-on-error: true` — it will activate automatically once secrets are added.

---

## What we already have

| Secret | Value | Source |
|---|---|---|
| `MSSTORE_SELLER_ID` | `15588cde-ee0a-4471-a7e9-d5828d87ada6` | Partner Center profile |
| `MSSTORE_TENANT_ID` | pending new tenant creation | Partner Center wizard |
| `MSSTORE_CLIENT_ID` | pending app registration | Partner Center Users page |
| `MSSTORE_CLIENT_SECRET` | pending app registration | Partner Center Users page |
| `MSSTORE_APP_ID` | pending | Partner Center → App identity |

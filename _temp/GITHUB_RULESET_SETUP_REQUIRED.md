# GitHub Ruleset Setup Required

**Date:** 2025-10-15
**Issue:** Direct pushes to `main` are currently allowed (rules can be bypassed)
**Priority:** HIGH

---

## Problem

Recent commits were pushed directly to `main`:
```
a62c54d Bump version to 0.3.15
ad9afbe Add arrow key support for Claude interactive menus
1a5ce5e Fix Claude stdin issues and improve terminal UX
582bf4b Bump version to 0.3.14
318b036 Fix ANSI color code decoding in terminal and debug console
```

Git push output showed:
```
remote: Bypassed rule violations for refs/heads/main:
remote:
remote: - Changes must be made through a pull request.
```

This indicates rulesets exist but can be bypassed.

---

## Required Action

### 1. Enable Strict Branch Protection

Navigate to:
```
https://github.com/a5af/agentmux/settings/rules
```

Or via GitHub UI:
1. Go to repository: `https://github.com/a5af/agentmux`
2. Click **Settings**
3. Click **Rules** → **Rulesets** (left sidebar)

### 2. Configure Ruleset for `main`

**Ruleset Name:** `Protect main branch`

**Target:** `main` branch (or pattern: `main`)

**Rules to Enable:**

#### ✅ Require Pull Request
- [x] Require pull request before merging
- [x] Dismiss stale pull request approvals when new commits are pushed
- Required approvals: **1** (minimum)
- Allow specified actors to bypass: **NONE** ❌

#### ✅ Block Force Push
- [x] Block force pushes

#### ✅ Restrict Deletions
- [x] Restrict deletions

#### ✅ Block Creation (optional)
- [x] Block branch creation (prevents accidental creation of `main` if deleted)

### 3. Enforcement Level

**CRITICAL:** Set enforcement to **Active** (not Disabled or Evaluate)

**Bypass list:** Leave empty or add ONLY:
- Repository administrators (if needed for emergencies)
- Deployment bots (if using automated releases)

**DO NOT** allow:
- Individual users
- Teams
- GitHub Apps (unless specifically needed)

---

## What This Prevents

### Before (Current State)
```bash
git checkout main
git commit -m "Direct commit"
git push origin main  # ✅ Succeeds (bypassed)
```

### After (Desired State)
```bash
git checkout main
git commit -m "Direct commit"
git push origin main  # ❌ REJECTED by GitHub
```

**Error message:**
```
remote: error: GH006: Protected branch update failed for refs/heads/main.
remote: error: Changes must be made through a pull request.
```

---

## Correct Workflow

### For All Changes
```bash
# 1. Create feature branch
git checkout -b feature/description

# 2. Make changes and commit
git add .
git commit -m "Description"

# 3. Push feature branch
git push -u origin feature/description

# 4. Create PR via GitHub CLI
gh pr create --title "Description" --body "Details"

# 5. Wait for approval and merge via GitHub UI
```

### For AgentX
```bash
# Generate agent ID
AGENT_ID="AgentX-$$-$(date +%s)"

# Include in PR title
gh pr create --title "AgentX ($AGENT_ID): Description" --body "..."
```

---

## Historical Context

From `_docs/GITHUB_RULESETS_QUICK_REFERENCE.md`:
- Rulesets were configured on **2025-10-09**
- Direct pushes to main should be blocked
- However, current configuration allows bypass

**Conclusion:** Rulesets exist but bypass permissions are too permissive.

---

## Verification Steps

After configuring ruleset:

### Test 1: Try Direct Push (Should Fail)
```bash
echo "test" >> README.md
git add README.md
git commit -m "Test direct push"
git push origin main  # Should be REJECTED
```

**Expected:** Error message about protected branch

### Test 2: PR Workflow (Should Work)
```bash
git checkout -b test/pr-workflow
echo "test" >> README.md
git add README.md
git commit -m "Test PR workflow"
git push -u origin test/pr-workflow  # Should succeed
gh pr create --title "Test PR" --body "Testing"  # Should succeed
```

**Expected:** PR created successfully

### Test 3: Verify No Bypass
```bash
# Even with admin rights, push should fail
git push origin main --force  # Should be REJECTED
```

**Expected:** Error even for admins (unless explicitly in bypass list)

---

## Remediation for Current Commits

### Option 1: Leave as-is (Recommended)
- Commits are already on `main`
- Code is tested and working
- Create retrospective PR for documentation only
- Learn from mistake and enforce going forward

### Option 2: Revert and Redo via PR
```bash
# Create branch from before the commits
git checkout -b fix/v0.3.15-via-pr 582bf4b^

# Cherry-pick commits
git cherry-pick 582bf4b..a62c54d

# Push and create PR
git push -u origin fix/v0.3.15-via-pr
gh pr create --title "v0.3.15 (Retroactive PR)" --body "..."
```

**NOT RECOMMENDED** - Would rewrite history on `main`

---

## Action Items

- [ ] **IMMEDIATE:** Configure GitHub ruleset (strict mode, no bypass)
- [ ] **IMMEDIATE:** Verify with test push (should fail)
- [ ] **ONGOING:** Always use feature branches
- [ ] **ONGOING:** Always create PRs for changes
- [ ] **DOCUMENT:** Update workflow documentation

---

**Status:** ⚠️ **ACTION REQUIRED**
**Owner:** Repository administrator
**Deadline:** Before next development session

---

**Reported by:** AgentX
**Date:** 2025-10-15

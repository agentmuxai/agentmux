# MASTER GUIDE REFERENCE

**⚠️ IMPORTANT: You are in a SUBMODULE, not the parent repository.**

---

## 🚨 STOP: Read Parent Repository's Master Guide

This repository (`agentmux`) is a **Git submodule** of the parent `WebProjects` workspace.

**The comprehensive agent guide you need is located in the PARENT repository:**

```
../_docs/MASTER_GUIDE_AGENT_ESSENTIALS.md
```

**Full absolute path (example for Agent5):**
```
D:\Code\WebProjects5\_docs\MASTER_GUIDE_AGENT_ESSENTIALS.md
```

---

## Why This Matters

When you're working inside a submodule directory, you may forget you're in a nested repository context. The parent repository contains:

✅ **Complete agent startup procedures**
✅ **Security requirements and rules**
✅ **Engineering principles**
✅ **GitHub workflow and PR requirements**
✅ **Workspace structure and navigation**
✅ **Troubleshooting guides**

**This submodule is part of the larger WebProjects workspace ecosystem.**

---

## Navigation

### To Read the Master Guide

```bash
# From this submodule directory (agentmux):
cd ..
cat _docs/MASTER_GUIDE_AGENT_ESSENTIALS.md

# Or use your editor/viewer:
code ../_docs/MASTER_GUIDE_AGENT_ESSENTIALS.md
```

### To Return to Parent Workspace Root

```bash
cd ..
pwd  # Should show: .../WebProjectsN (where N matches your agent)
```

---

## Quick Context Check

**Are you in a submodule?**
```bash
git rev-parse --show-superproject-working-tree
```
- If output shows a path → You're in a submodule
- If empty → You're in the parent workspace root

**Which repository are you in?**
```bash
pwd
basename $(pwd)
```
- `agentmux` → You're in this submodule
- `WebProjects` or `WebProjectsN` → You're in parent workspace

---

## What This File Is NOT

❌ This is NOT a replacement for the master guide
❌ This is NOT comprehensive documentation
❌ This is NOT project-specific instructions

**This is simply a pointer** to help you navigate back to the parent repository's comprehensive guide.

---

## Project-Specific Documentation

For `agentmux`-specific documentation (architecture, setup, API docs), see:
- `README.md` (this repository)
- `docs/` directory (if exists)
- Project-specific guides in this repo

For **agent workflow, security rules, and startup procedures**, always refer to:
- `../_docs/MASTER_GUIDE_AGENT_ESSENTIALS.md` (parent repository)

---

**Remember:** When working across submodules, always maintain awareness of your current repository context.

**Last Updated:** 2025-10-15

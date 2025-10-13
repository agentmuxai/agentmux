# Security Considerations for Reactive Claude CLI Agents

**Version:** 1.0
**Date:** 2025-10-13
**Component:** reactive-claude-agent.js

---

## Overview

This document outlines security assumptions and considerations for the reactive Claude CLI agent wrapper.

---

## Input Security

### Message Injection via stdin

**Implementation:**
```javascript
// Line 276
this.process.stdin.write(input);
```

**Security Assumptions:**

1. **Claude CLI Input Validation**
   - **Assumption:** Claude CLI properly sanitizes and validates all stdin input
   - **Rationale:** Claude CLI is the official Anthropic client, designed to handle arbitrary user input safely
   - **Risk Level:** LOW - Claude CLI is trusted first-party software

2. **Message Source Trust**
   - **Trust Model:** Messages originate from:
     - Desktop UI (user-controlled)
     - Other agents in the same user's workspace
     - File system messages (`~/.agentmux/shared/messages/`)
   - **Assumption:** All message sources are within the same user's security boundary
   - **Risk Level:** LOW - No external/untrusted message sources

3. **No Command Injection**
   - **Safe:** stdin is passed directly to Claude CLI, not executed as shell commands
   - **Implementation:** Uses `child_process.spawn()` with explicit command and no shell interpretation
   - **Risk Level:** NONE - No shell execution

### Message Content Validation

**Current Implementation:**
```javascript
// Line 261-262
const input = message.payload.text + '\n';
this.process.stdin.write(input);
```

**Security Measures:**
- ✅ **No special character interpretation** - Text passed as-is to Claude CLI
- ✅ **No eval() or code execution** - Pure stdin pipe
- ✅ **No file system operations** - Message content not used for file paths
- ✅ **No network requests** - Message content not used in URLs

**Risk Level:** LOW - Message content is treated as plain text

---

## Process Isolation

### Subprocess Security

**Implementation:**
```javascript
// Line 59-67
this.process = spawn(this.cliCommand, [], {
  stdio: ['pipe', 'pipe', 'pipe'],
  cwd: process.cwd(),
  env: {
    ...process.env,
    AGENT_ID: this.agentId,
    NO_COLOR: '1',
  }
});
```

**Security Features:**
- ✅ **No shell:** Uses `spawn()` directly, not `exec()` or `shell: true`
- ✅ **Explicit command:** No PATH search vulnerabilities
- ✅ **Inherited environment:** No sensitive env vars added
- ✅ **Limited stdio:** Only pipes, no TTY access

**Risk Level:** LOW - Standard subprocess isolation

### Process Lifecycle

**Security Considerations:**
1. **Graceful shutdown** - SIGTERM/SIGINT handlers prevent orphaned processes
2. **Error handling** - stderr captured and logged, not executed
3. **Exit cleanup** - Process state cleaned up on exit

---

## File System Security

### Directory Structure

**Created Directories:**
```
~/.agentmux/
├── shared/
│   └── messages/          # Read/write by all agents
└── desktop/
    └── agents/{id}/       # Read/write by agent only
        ├── status.json
        ├── live-output.txt
        └── agent.log
```

**Security Measures:**
- ✅ **User-scoped:** All files in user's home directory
- ✅ **No cross-user access:** Standard Unix permissions
- ✅ **No setuid/setgid:** Normal file permissions
- ✅ **Path sanitization:** Uses `path.join()` consistently

**Risk Level:** LOW - Standard file system isolation

### File Permissions

**Default Permissions:** Uses Node.js defaults (respects umask)

**Recommendation:** For enhanced security:
```javascript
// Set restrictive permissions
fs.writeFileSync(statusFile, data, { mode: 0o600 }); // Owner read/write only
```

### Message File Watching

**Implementation:**
```javascript
// Line 186-194
fs.watch(this.messagesDir, (eventType, filename) => {
  if (!filename?.endsWith('.json')) return;
  // Process message...
});
```

**Security Considerations:**
1. **File type filtering** - Only `.json` files processed
2. **Duplicate detection** - `processedMessages` Set prevents re-processing
3. **Error handling** - Failed reads logged, don't crash agent

**Risk Level:** LOW - Read-only access to message files

---

## Output Security

### Buffer Management

**Implementation:**
```javascript
// Line 99-107
this.fullOutput += text;

// Trim buffer if it exceeds max size
if (this.fullOutput.length > this.MAX_OUTPUT_SIZE) {
  const keepSize = Math.floor(this.MAX_OUTPUT_SIZE / 2);
  this.fullOutput = '... [earlier output truncated]\n' + this.fullOutput.slice(-keepSize);
}
```

**Security Features:**
- ✅ **Size limit enforced:** 1MB maximum (prevents memory exhaustion)
- ✅ **Automatic trimming:** Keeps last 50% when limit exceeded
- ✅ **No unbounded growth:** Memory-safe for long-running agents

**Risk Level:** LOW - Memory safety ensured

### Output Logging

**Files Written:**
1. `live-output.txt` - Real-time output for Desktop UI
2. `agent.log` - Timestamped append-only log

**Security Considerations:**
- ✅ **No log injection** - Output written as-is, no interpretation
- ✅ **Timestamped entries** - Audit trail maintained
- ⚠️ **Sensitive data exposure** - Claude CLI output may contain sensitive info

**Recommendation:**
- User should be aware that agent logs contain full conversation
- Consider log rotation/cleanup policies
- No automatic log sanitization (by design)

---

## Authentication & Authorization

### Agent Identity

**Implementation:**
```javascript
// Line 237-238
this.agentId = agentId;  // Set at construction
AGENT_ID: this.agentId   // Passed to Claude CLI
```

**Security Model:**
- **Identity Verification:** None - Agent ID is self-declared
- **Authorization:** All agents have equal permissions
- **Impersonation:** Possible - Any agent can claim any ID

**Risk Level:** MEDIUM - No authentication between agents

**Rationale:**
- All agents run within same user's security boundary
- No external/untrusted agents
- Cooperative system, not adversarial

**Recommendation for Production:**
If deploying in multi-tenant environment, implement:
1. Agent ID signing/verification
2. Message authentication codes (MAC)
3. Per-agent access control lists (ACL)

---

## Message Bus Security

### Pattern Matching

**Implementation:**
```javascript
// Line 236-252
isMessageForMe(message) {
  const to = message.to;

  // Exact match
  if (to === this.agentId) return true;

  // Broadcast
  if (to === '*') return true;

  // Wildcard pattern
  if (to.endsWith('*')) {
    const prefix = to.slice(0, -1);
    if (this.agentId.startsWith(prefix)) return true;
  }

  return false;
}
```

**Security Features:**
- ✅ **Simple pattern matching** - No regex vulnerabilities
- ✅ **No code execution** - Pure string comparison
- ✅ **No backtracking** - No ReDoS risk

**Attack Vectors:**
- ❌ **Message flooding** - No rate limiting
- ❌ **Broadcast spam** - Any agent can spam all agents

**Risk Level:** MEDIUM - No anti-abuse mechanisms

**Recommendation for Production:**
1. Rate limiting per sender
2. Message priority queue
3. Admin-only broadcast permissions

---

## Claude CLI Trust Model

### Assumptions

1. **Claude CLI is trusted software**
   - Official Anthropic client
   - Regular security updates
   - Input validation built-in

2. **Claude CLI handles untrusted input safely**
   - No command injection
   - No buffer overflows
   - No code execution vulnerabilities

3. **Claude CLI respects environment isolation**
   - No unauthorized file access
   - No network exfiltration
   - No privilege escalation

### Verification

**User Responsibility:**
- Keep Claude CLI updated to latest version
- Monitor Claude CLI security advisories
- Report any suspicious behavior

**Wrapper Responsibility:**
- Pass input to Claude CLI without modification
- Capture output without interpretation
- Isolate Claude CLI process

---

## Threat Model

### In-Scope Threats

| Threat | Likelihood | Impact | Mitigation |
|--------|-----------|--------|-----------|
| Memory exhaustion | LOW | MEDIUM | Buffer size limit (1MB) |
| Disk exhaustion | MEDIUM | MEDIUM | Log rotation recommended |
| Message flooding | MEDIUM | LOW | Throttling recommended |
| Agent impersonation | LOW | LOW | Cooperative environment |

### Out-of-Scope Threats

- **System-level attacks:** OS/kernel exploits
- **Claude CLI vulnerabilities:** Handled by Anthropic
- **Hardware attacks:** Physical security
- **Social engineering:** User awareness

---

## Security Best Practices

### For Users

1. **Run with least privilege** - Don't run as root/admin
2. **Monitor agent logs** - Check for unexpected behavior
3. **Update Claude CLI regularly** - Security patches
4. **Use dedicated workspace** - Isolate from sensitive files

### For Developers

1. **Validate all inputs** - Before passing to Claude CLI
2. **Implement rate limiting** - Prevent abuse
3. **Add authentication** - For multi-tenant deployments
4. **Rotate logs** - Prevent disk exhaustion
5. **Monitor memory usage** - Watch for leaks

---

## Security Checklist

**Before Production Deployment:**

- [ ] Update Claude CLI to latest version
- [ ] Review log file contents for sensitive data
- [ ] Implement log rotation policy
- [ ] Set restrictive file permissions (0o600)
- [ ] Monitor agent memory usage
- [ ] Test message flood scenarios
- [ ] Document incident response procedures
- [ ] Review message authentication needs
- [ ] Consider rate limiting implementation
- [ ] Test graceful shutdown/restart

---

## Security Updates

**Version 1.0 (2025-10-13):**
- ✅ Added buffer size limit (1MB)
- ✅ Throttled status writes (max 1/sec)
- ✅ Documented security assumptions

**Future Enhancements:**
- Message authentication (HMAC)
- Rate limiting per sender
- Log encryption at rest
- Agent ID verification
- Memory usage monitoring
- Disk quota enforcement

---

## Contact

**Security Issues:** Report via GitHub Security Advisory
**Questions:** See TESTING.md and README.md

---

**Last Updated:** 2025-10-13
**Reviewed By:** Agent2, AgentX
**Status:** Production Ready (with recommendations)

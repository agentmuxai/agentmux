// Configure core.hooksPath, but only inside a real git working tree.
// A bare `git config core.hooksPath .githooks` in package.json's "prepare"
// script exits 128 ("fatal: not a git repository") in any packaging context
// that has package.json + lockfile but no .git directory -- a Docker
// dependency-caching layer, a Lambda/serverless zip step, etc. -- which
// would hard-fail `npm install`/`npm ci` there. Node is guaranteed present
// in any npm context and behaves identically across platforms, unlike a
// shell one-liner (see the sibling GNU-sed / cmd.exe issues this hook
// already hit once).
const { execSync } = require('child_process');

try {
  execSync('git rev-parse --git-dir', { stdio: 'ignore' });
} catch {
  // Not inside a git working tree -- nothing to configure, exit 0 quietly.
  process.exit(0);
}

execSync('git config core.hooksPath .githooks', { stdio: 'ignore' });

/**
 * GitHub Pull Request Merge Event Handler
 *
 * Detects when a PR is merged via the GitHub web UI and notifies
 * the PR author (if they are an agent) via AgentMux inject_terminal.
 */

import { isKnownAgent, getAgentId } from '../agent-mapping.js';

/**
 * GitHub Pull Request webhook event payload (simplified)
 */
export interface PullRequestEvent {
  action: string;
  pull_request: {
    number: number;
    title: string;
    html_url: string;
    merged: boolean;
    merged_by?: {
      login: string;
    };
    user: {
      login: string;
    };
    head: {
      ref: string;
    };
    base: {
      ref: string;
    };
  };
}

/**
 * Check if a merge was done via GitHub web UI.
 *
 * Web UI merges have:
 * - committer.login == "web-flow" (GitHub's web UI bot)
 *
 * Note: We can't check committer in the webhook payload directly,
 * but we can infer it's a web merge if:
 * - merged_by is different from the PR author
 * - The merge happened (not closed without merge)
 */
export function isWebMerge(event: PullRequestEvent): boolean {
  const { pull_request: pr } = event;

  // Must be a closed PR that was actually merged
  if (event.action !== 'closed' || !pr.merged) {
    return false;
  }

  // If author merged their own PR, don't notify
  if (pr.merged_by?.login === pr.user.login) {
    return false;
  }

  // For now, we'll notify on all merges where someone else merged
  // In the future, we could check the commit to verify web-flow committer
  return true;
}

/**
 * Format the merge notification message for the agent.
 */
export function formatMergeNotification(event: PullRequestEvent): string {
  const { pull_request: pr } = event;
  const mergedBy = pr.merged_by?.login || 'unknown';

  return `MERGED | PR #${pr.number} merged by @${mergedBy}

Title: ${pr.title}
Branch: ${pr.head.ref} -> ${pr.base.ref}
URL: ${pr.html_url}

Next steps:
- Clean up branch: git checkout main && git pull && git branch -d ${pr.head.ref}
- Deploy if needed: claw deploy`;
}

export interface MergeHandlerResult {
  shouldNotify: boolean;
  targetAgentId?: string;
  message?: string;
  reason?: string;
}

/**
 * Process a pull_request event and determine if notification is needed.
 */
export function processMergeEvent(event: PullRequestEvent): MergeHandlerResult {
  // Check if this is a merge event we care about
  if (event.action !== 'closed') {
    return { shouldNotify: false, reason: 'Not a closed PR event' };
  }

  if (!event.pull_request.merged) {
    return { shouldNotify: false, reason: 'PR was closed without merging' };
  }

  const authorLogin = event.pull_request.user.login;
  const mergedByLogin = event.pull_request.merged_by?.login;

  // Author merged their own PR - no notification needed
  if (authorLogin === mergedByLogin) {
    return { shouldNotify: false, reason: 'Author merged their own PR' };
  }

  // Check if author is a known agent
  if (!isKnownAgent(authorLogin)) {
    return { shouldNotify: false, reason: `Author ${authorLogin} is not a known agent` };
  }

  const agentId = getAgentId(authorLogin);
  if (!agentId) {
    return { shouldNotify: false, reason: `Could not map ${authorLogin} to agent ID` };
  }

  return {
    shouldNotify: true,
    targetAgentId: agentId,
    message: formatMergeNotification(event),
  };
}

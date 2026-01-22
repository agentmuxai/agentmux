/**
 * GitHub Check Run Failure Event Handler
 *
 * Sends jekt notifications when CI fails on an agent's PR.
 * Extracts the agent ID from the PR author's GitHub identity.
 */

import { getAgentId } from '../agent-mapping.js';

/**
 * GitHub Check Run webhook event payload (simplified)
 */
export interface CheckRunEvent {
  action: string;
  check_run: {
    name: string;
    conclusion: string | null;
    html_url: string;
    pull_requests: Array<{
      number: number;
      head: {
        ref: string;
      };
    }>;
  };
  repository: {
    full_name: string;
  };
}

/**
 * PR details from GitHub API (for author info)
 */
export interface PullRequestDetails {
  number: number;
  title: string;
  user: {
    login: string;
  };
}

export interface CIFailureHandlerResult {
  shouldNotify: boolean;
  targetAgentId?: string;
  message?: string;
  reason?: string;
  prNumber?: number;
}

/**
 * Process a check_run event and determine if notification is needed.
 *
 * @param event - GitHub webhook payload
 * @param prDetails - Optional: PR details fetched from GitHub API
 */
export function processCIFailureEvent(
  event: CheckRunEvent,
  prDetails?: PullRequestDetails
): CIFailureHandlerResult {
  const { action, check_run: checkRun, repository } = event;

  // Only process completed failures
  if (action !== 'completed') {
    return { shouldNotify: false, reason: `Action is '${action}', not 'completed'` };
  }

  if (checkRun.conclusion !== 'failure') {
    return { shouldNotify: false, reason: `Conclusion is '${checkRun.conclusion}', not 'failure'` };
  }

  // Get associated PRs
  const pullRequests = checkRun.pull_requests || [];
  if (pullRequests.length === 0) {
    return { shouldNotify: false, reason: 'No associated PRs' };
  }

  // Process first PR (usually only one)
  const pr = pullRequests[0];
  const prNumber = pr.number;
  const branch = pr.head.ref;

  // If we don't have PR details, we can't determine the author
  if (!prDetails) {
    return {
      shouldNotify: false,
      reason: 'PR details not provided - cannot determine author',
      prNumber,
    };
  }

  // Map GitHub author to agent ID
  const authorLogin = prDetails.user.login;
  const agentId = getAgentId(authorLogin);

  if (!agentId) {
    return {
      shouldNotify: false,
      reason: `Author '${authorLogin}' is not an agent`,
      prNumber,
    };
  }

  console.log(`CI failure: ${repository.full_name}#${prNumber} (${checkRun.name}) -> ${agentId}`);

  return {
    shouldNotify: true,
    targetAgentId: agentId,
    prNumber,
    message: formatCIFailureNotification(prNumber, prDetails.title, checkRun.name, branch, checkRun.html_url),
  };
}

/**
 * Format the CI failure notification message for the agent.
 */
function formatCIFailureNotification(
  prNumber: number,
  prTitle: string,
  checkName: string,
  branch: string,
  detailsUrl: string
): string {
  return `[FAIL] PR #${prNumber} FAILED CI

Title: ${prTitle}
Check: ${checkName}
Branch: ${branch}
Details: ${detailsUrl}

Fix the failing tests and push again.`;
}

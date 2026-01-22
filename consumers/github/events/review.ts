/**
 * GitHub Pull Request Review Event Handler
 *
 * Sends jekt notifications when a PR is reviewed:
 * 1. To PR author (if agent)
 * 2. To most recent committer (if agent and different from author)
 *
 * This enables agents to receive notifications when:
 * - Their PR is reviewed
 * - A PR they contributed to is reviewed (even if they're not the author)
 */

import { getAgentId } from '../agent-mapping.js';

/**
 * GitHub Pull Request Review webhook event payload (simplified)
 */
export interface PullRequestReviewEvent {
  action: string;
  review: {
    state: string;
    html_url: string;
    user: {
      login: string;
    };
  };
  pull_request: {
    number: number;
    title: string;
    html_url: string;
    user: {
      login: string;
    };
    head: {
      ref: string;
      sha: string;
    };
  };
  repository: {
    full_name: string;
  };
}

// Review states that trigger notifications
const ENABLED_REVIEW_STATES = ['approved', 'changes_requested'];

export interface ReviewHandlerResult {
  shouldNotify: boolean;
  targetAgentIds: string[];
  message?: string;
  reason?: string;
}

/**
 * Process a pull_request_review event and determine if notification is needed.
 *
 * @param event - GitHub webhook payload
 * @param headCommitAuthor - Optional: login of the head commit author (fetched separately)
 */
export function processReviewEvent(
  event: PullRequestReviewEvent,
  headCommitAuthor?: string
): ReviewHandlerResult {
  // Only process submitted reviews
  if (event.action !== 'submitted') {
    return { shouldNotify: false, targetAgentIds: [], reason: 'Not a submitted review' };
  }

  const reviewState = event.review.state.toLowerCase();

  // Filter by review state
  if (!ENABLED_REVIEW_STATES.includes(reviewState)) {
    return { shouldNotify: false, targetAgentIds: [], reason: `Review state '${reviewState}' not enabled` };
  }

  // Collect agents to notify (using Set to deduplicate)
  const agentsToNotify = new Set<string>();

  // 1. Check PR author
  const prAuthorLogin = event.pull_request.user.login;
  const prAuthorAgent = getAgentId(prAuthorLogin);
  if (prAuthorAgent) {
    agentsToNotify.add(prAuthorAgent);
    console.log(`PR author is agent: ${prAuthorLogin} -> ${prAuthorAgent}`);
  }

  // 2. Check most recent committer (if provided)
  if (headCommitAuthor) {
    const committerAgent = getAgentId(headCommitAuthor);
    if (committerAgent) {
      agentsToNotify.add(committerAgent);
      console.log(`Head commit author is agent: ${headCommitAuthor} -> ${committerAgent}`);
    }
  }

  // Skip if no agents to notify
  if (agentsToNotify.size === 0) {
    return {
      shouldNotify: false,
      targetAgentIds: [],
      reason: `No agents to notify: author=${prAuthorLogin}, committer=${headCommitAuthor || 'unknown'}`,
    };
  }

  return {
    shouldNotify: true,
    targetAgentIds: Array.from(agentsToNotify),
    message: formatReviewNotification(event),
  };
}

/**
 * Format the review notification message for the agent.
 */
function formatReviewNotification(event: PullRequestReviewEvent): string {
  const { review, pull_request: pr } = event;
  const reviewerLogin = review.user.login;
  const reviewState = review.state.toLowerCase();

  let emoji: string;
  let status: string;
  let action: string;

  if (reviewState === 'approved') {
    emoji = '[OK]';
    status = 'APPROVED';
    action = 'Your PR has been approved! Ready to merge.';
  } else if (reviewState === 'changes_requested') {
    emoji = '[!!]';
    status = 'Changes requested';
    action = 'Review the feedback and update your PR.';
  } else {
    emoji = '[--]';
    status = 'Reviewed';
    action = 'Check the review comments.';
  }

  return `${emoji} PR #${pr.number} ${status} by @${reviewerLogin}

Title: ${pr.title}
Review: ${review.html_url}
Branch: ${pr.head.ref}

${action}`;
}

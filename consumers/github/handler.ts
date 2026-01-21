/**
 * AgentMux GitHub Consumer Lambda Handler
 *
 * Receives GitHub webhook events via:
 * - SNS subscription (from github-router fan-out)
 * - Direct API Gateway (legacy, for merge notifications)
 *
 * Currently handles:
 * - pull_request.closed (merged) - Notify PR author when their PR is merged
 * - pull_request_review - Notify PR author/committer when PR is reviewed
 * - check_run (failure) - Notify PR author when CI fails
 */

import type { APIGatewayProxyEventV2, APIGatewayProxyResultV2, SNSEvent } from 'aws-lambda';
import { createHmac, timingSafeEqual } from 'crypto';
import { SecretsManagerClient, GetSecretValueCommand } from '@aws-sdk/client-secrets-manager';
import { processMergeEvent, PullRequestEvent } from './events/merge.js';
import { processReviewEvent, PullRequestReviewEvent } from './events/review.js';
import { processCIFailureEvent, CheckRunEvent, PullRequestDetails } from './events/ci-failure.js';

// Environment variables
const AGENTMUX_URL = process.env.AGENTMUX_URL || 'https://agentmux.asaf.cc';

// Secrets Manager client (reused across invocations)
const secretsClient = new SecretsManagerClient({});

// Cached secrets (Lambda container reuse)
let cachedWebhookSecret: string | null = null;
let cachedApiKey: string | null = null;
let cachedGithubToken: string | null = null;

/**
 * Fetch secret from AWS Secrets Manager.
 */
async function getSecret(secretPath: string): Promise<string> {
  const command = new GetSecretValueCommand({ SecretId: secretPath });
  const response = await secretsClient.send(command);

  if (!response.SecretString) {
    throw new Error(`Secret ${secretPath} has no string value`);
  }

  return response.SecretString;
}

/**
 * Get GitHub webhook secret from Secrets Manager.
 */
async function getWebhookSecret(): Promise<string> {
  if (cachedWebhookSecret) return cachedWebhookSecret;

  const secretJson = await getSecret('services/infra');
  const secrets = JSON.parse(secretJson);
  cachedWebhookSecret = secrets['github-webhook-secret'] || '';

  if (!cachedWebhookSecret) {
    console.warn('github-webhook-secret not found in services/infra');
  }

  return cachedWebhookSecret;
}

/**
 * Get AgentMux API key from Secrets Manager.
 */
async function getApiKey(): Promise<string> {
  if (cachedApiKey) return cachedApiKey;

  const secretJson = await getSecret('services/infra');
  const secrets = JSON.parse(secretJson);
  // Try both key names for compatibility
  cachedApiKey = secrets['agentmux-api-key'] || secrets.agentmux?.token || '';

  if (!cachedApiKey) {
    throw new Error('agentmux-api-key not found in services/infra');
  }

  return cachedApiKey;
}

/**
 * Get GitHub token from Secrets Manager.
 */
async function getGithubToken(): Promise<string> {
  if (cachedGithubToken) return cachedGithubToken;

  const secretJson = await getSecret('services/infra');
  const secrets = JSON.parse(secretJson);
  cachedGithubToken = secrets.github?.token || '';

  if (!cachedGithubToken) {
    throw new Error('github.token not found in services/infra');
  }

  return cachedGithubToken;
}

/**
 * Verify GitHub webhook signature using timing-safe comparison.
 */
async function verifySignature(payload: string, signature: string | undefined): Promise<boolean> {
  const webhookSecret = await getWebhookSecret();

  if (!webhookSecret) {
    console.warn('GITHUB_WEBHOOK_SECRET not configured - skipping signature verification');
    return true;
  }

  if (!signature) {
    console.error('No X-Hub-Signature-256 header');
    return false;
  }

  const expectedSignature = `sha256=${createHmac('sha256', webhookSecret)
    .update(payload)
    .digest('hex')}`;

  // Use timing-safe comparison to prevent timing attacks
  if (signature.length !== expectedSignature.length) {
    return false;
  }

  return timingSafeEqual(
    Buffer.from(signature, 'utf8'),
    Buffer.from(expectedSignature, 'utf8')
  );
}

/**
 * Send injection to AgentMux.
 */
async function injectToAgent(targetAgent: string, message: string, prNumber?: number): Promise<void> {
  const apiKey = await getApiKey();

  const body: Record<string, unknown> = {
    target_agent: targetAgent,
    message: message,
    priority: 'urgent',
    source_agent: 'github-consumer',
  };

  if (prNumber !== undefined) {
    body.pr_number = prNumber;
  }

  const response = await fetch(`${AGENTMUX_URL}/reactive/inject`, {
    method: 'POST',
    headers: {
      'Content-Type': 'application/json',
      'Authorization': `Bearer ${apiKey}`,
      'X-Agent-ID': 'github-consumer',
    },
    body: JSON.stringify(body),
  });

  if (!response.ok) {
    const errorText = await response.text();
    throw new Error(`AgentMux injection failed: ${response.status} ${errorText}`);
  }

  const result = await response.json();
  console.log('Injection created:', result);
}

/**
 * Fetch commit author from GitHub API.
 */
async function fetchCommitAuthor(repo: string, sha: string): Promise<string | undefined> {
  try {
    const token = await getGithubToken();
    const response = await fetch(`https://api.github.com/repos/${repo}/commits/${sha}`, {
      headers: {
        'Authorization': `token ${token}`,
        'Accept': 'application/vnd.github.v3+json',
      },
    });

    if (!response.ok) {
      console.warn(`Failed to fetch commit ${sha}: ${response.status}`);
      return undefined;
    }

    const commit = await response.json() as { author?: { login?: string } };
    return commit.author?.login;
  } catch (error) {
    console.warn(`Error fetching commit ${sha}:`, error);
    return undefined;
  }
}

/**
 * Fetch PR details from GitHub API.
 */
async function fetchPRDetails(repo: string, prNumber: number): Promise<PullRequestDetails | undefined> {
  try {
    const token = await getGithubToken();
    const response = await fetch(`https://api.github.com/repos/${repo}/pulls/${prNumber}`, {
      headers: {
        'Authorization': `token ${token}`,
        'Accept': 'application/vnd.github.v3+json',
      },
    });

    if (!response.ok) {
      console.warn(`Failed to fetch PR ${repo}#${prNumber}: ${response.status}`);
      return undefined;
    }

    return await response.json() as PullRequestDetails;
  } catch (error) {
    console.warn(`Error fetching PR ${repo}#${prNumber}:`, error);
    return undefined;
  }
}

/**
 * Process a GitHub event (from either API Gateway or SNS).
 */
async function processGitHubEvent(eventType: string, payload: unknown): Promise<{ processed: boolean; notifications: number; reason?: string }> {
  let notifications = 0;

  switch (eventType) {
    case 'pull_request': {
      const prEvent = payload as PullRequestEvent;
      console.log('Processing pull_request event:', {
        action: prEvent.action,
        number: prEvent.pull_request?.number,
        merged: prEvent.pull_request?.merged,
        author: prEvent.pull_request?.user?.login,
      });

      const result = processMergeEvent(prEvent);
      console.log('Merge handler result:', result);

      if (result.shouldNotify && result.targetAgentId && result.message) {
        await injectToAgent(result.targetAgentId, result.message, prEvent.pull_request?.number);
        console.log(`Notification sent to ${result.targetAgentId}`);
        notifications++;
      }

      return { processed: true, notifications, reason: result.reason };
    }

    case 'pull_request_review': {
      const reviewEvent = payload as PullRequestReviewEvent;
      console.log('Processing pull_request_review event:', {
        action: reviewEvent.action,
        state: reviewEvent.review?.state,
        prNumber: reviewEvent.pull_request?.number,
        author: reviewEvent.pull_request?.user?.login,
        reviewer: reviewEvent.review?.user?.login,
      });

      // Fetch head commit author for committer jekt feature
      const repo = reviewEvent.repository?.full_name;
      const headSha = reviewEvent.pull_request?.head?.sha;
      let headCommitAuthor: string | undefined;

      if (repo && headSha) {
        headCommitAuthor = await fetchCommitAuthor(repo, headSha);
        console.log(`Head commit author: ${headCommitAuthor || 'unknown'}`);
      }

      const result = processReviewEvent(reviewEvent, headCommitAuthor);
      console.log('Review handler result:', result);

      if (result.shouldNotify && result.message) {
        for (const agentId of result.targetAgentIds) {
          await injectToAgent(agentId, result.message, reviewEvent.pull_request?.number);
          console.log(`Notification sent to ${agentId}`);
          notifications++;
        }
      }

      return { processed: true, notifications, reason: result.reason };
    }

    case 'check_run': {
      const checkRunEvent = payload as CheckRunEvent;
      console.log('Processing check_run event:', {
        action: checkRunEvent.action,
        conclusion: checkRunEvent.check_run?.conclusion,
        name: checkRunEvent.check_run?.name,
        prCount: checkRunEvent.check_run?.pull_requests?.length,
      });

      // Only process failures
      if (checkRunEvent.action !== 'completed' || checkRunEvent.check_run?.conclusion !== 'failure') {
        return { processed: true, notifications: 0, reason: 'Not a failure' };
      }

      // Fetch PR details to get author
      const repo = checkRunEvent.repository?.full_name;
      const prNumber = checkRunEvent.check_run?.pull_requests?.[0]?.number;
      let prDetails: PullRequestDetails | undefined;

      if (repo && prNumber) {
        prDetails = await fetchPRDetails(repo, prNumber);
      }

      const result = processCIFailureEvent(checkRunEvent, prDetails);
      console.log('CI failure handler result:', result);

      if (result.shouldNotify && result.targetAgentId && result.message) {
        await injectToAgent(result.targetAgentId, result.message, result.prNumber);
        console.log(`Notification sent to ${result.targetAgentId}`);
        notifications++;
      }

      return { processed: true, notifications, reason: result.reason };
    }

    case 'ping': {
      console.log('Ping event received - webhook configured successfully');
      return { processed: true, notifications: 0 };
    }

    default: {
      console.log(`Unhandled event type: ${eventType}`);
      return { processed: false, notifications: 0, reason: `Unhandled event type: ${eventType}` };
    }
  }
}

/**
 * Handle SNS events (from github-router fan-out).
 */
async function handleSNSEvent(event: SNSEvent): Promise<{ statusCode: number; body: string }> {
  let processed = 0;
  let notifications = 0;

  for (const record of event.Records) {
    try {
      // Parse SNS message (router format)
      const snsMessage = JSON.parse(record.Sns.Message) as {
        event_type: string;
        delivery_id: string;
        payload: unknown;
      };

      const eventType = snsMessage.event_type;
      const deliveryId = snsMessage.delivery_id || 'unknown';
      const payload = snsMessage.payload;

      console.log(`Processing SNS message: ${eventType} (${deliveryId})`);

      const result = await processGitHubEvent(eventType, payload);
      if (result.processed) {
        processed++;
        notifications += result.notifications;
      }
    } catch (error) {
      console.error('Error processing SNS record:', error);
    }
  }

  console.log(`SNS processing complete: processed=${processed}, notifications=${notifications}`);

  return {
    statusCode: 200,
    body: JSON.stringify({ processed, notifications }),
  };
}

/**
 * Handle API Gateway events (direct webhook).
 */
async function handleAPIGatewayEvent(event: APIGatewayProxyEventV2): Promise<APIGatewayProxyResultV2> {
  console.log('API Gateway webhook received');

  // Verify webhook signature
  const signature = event.headers['x-hub-signature-256'];
  const body = event.body || '';

  if (!await verifySignature(body, signature)) {
    console.error('Invalid webhook signature');
    return {
      statusCode: 401,
      body: JSON.stringify({ error: 'Invalid signature' }),
    };
  }

  // Parse event
  let payload: unknown;
  try {
    payload = JSON.parse(body);
  } catch (error) {
    console.error('Failed to parse webhook body:', error);
    return {
      statusCode: 400,
      body: JSON.stringify({ error: 'Invalid JSON body' }),
    };
  }

  // Get event type from header
  const eventType = event.headers['x-github-event'] || 'unknown';
  console.log('Event type:', eventType);

  try {
    const result = await processGitHubEvent(eventType, payload);

    return {
      statusCode: 200,
      body: JSON.stringify({
        processed: result.processed,
        eventType,
        notifications: result.notifications,
        reason: result.reason,
      }),
    };
  } catch (error) {
    console.error('Error processing webhook:', error);
    return {
      statusCode: 500,
      body: JSON.stringify({ error: 'Internal server error', message: (error as Error).message }),
    };
  }
}

/**
 * Main Lambda handler - supports both SNS and API Gateway events.
 */
export async function handler(
  event: SNSEvent | APIGatewayProxyEventV2
): Promise<APIGatewayProxyResultV2 | { statusCode: number; body: string }> {
  // Detect event type
  if ('Records' in event && event.Records?.[0]?.EventSource === 'aws:sns') {
    return handleSNSEvent(event as SNSEvent);
  } else {
    return handleAPIGatewayEvent(event as APIGatewayProxyEventV2);
  }
}

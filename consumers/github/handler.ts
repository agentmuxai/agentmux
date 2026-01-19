/**
 * AgentMux GitHub Consumer Lambda Handler
 *
 * Receives GitHub webhook events and notifies agents via AgentMux.
 * Currently handles:
 * - pull_request.closed (merged) - Notify PR author when their PR is merged
 *
 * Future handlers:
 * - @agent mentions in PR comments
 * - Issue assignments to agents
 * - PR review requests
 */

import type { APIGatewayProxyEventV2, APIGatewayProxyResultV2 } from 'aws-lambda';
import { createHmac, timingSafeEqual } from 'crypto';
import { SecretsManagerClient, GetSecretValueCommand } from '@aws-sdk/client-secrets-manager';
import { processMergeEvent, PullRequestEvent } from './events/merge.js';

// Environment variables
const AGENTMUX_URL = process.env.AGENTMUX_URL || 'https://agentmux.asaf.cc';

// Secrets Manager client (reused across invocations)
const secretsClient = new SecretsManagerClient({});

// Cached secrets (Lambda container reuse)
let cachedWebhookSecret: string | null = null;
let cachedApiKey: string | null = null;

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
  cachedApiKey = secrets['agentmux-api-key'] || '';

  if (!cachedApiKey) {
    throw new Error('agentmux-api-key not found in services/infra');
  }

  return cachedApiKey;
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
async function injectToAgent(targetAgent: string, message: string): Promise<void> {
  const apiKey = await getApiKey();

  const response = await fetch(`${AGENTMUX_URL}/reactive/inject`, {
    method: 'POST',
    headers: {
      'Content-Type': 'application/json',
      'Authorization': `Bearer ${apiKey}`,
      'X-Agent-ID': 'github-consumer',
    },
    body: JSON.stringify({
      target_agent: targetAgent,
      message: message,
      priority: 'normal',
    }),
  });

  if (!response.ok) {
    const errorText = await response.text();
    throw new Error(`AgentMux injection failed: ${response.status} ${errorText}`);
  }

  const result = await response.json();
  console.log('Injection created:', result);
}

/**
 * Main Lambda handler.
 */
export async function handler(
  event: APIGatewayProxyEventV2
): Promise<APIGatewayProxyResultV2> {
  console.log('GitHub webhook received:', {
    headers: event.headers,
    requestContext: event.requestContext,
  });

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
  const eventType = event.headers['x-github-event'];
  console.log('Event type:', eventType);

  try {
    switch (eventType) {
      case 'pull_request': {
        const prEvent = payload as PullRequestEvent;
        console.log('Processing pull_request event:', {
          action: prEvent.action,
          number: prEvent.pull_request?.number,
          merged: prEvent.pull_request?.merged,
          author: prEvent.pull_request?.user?.login,
          merged_by: prEvent.pull_request?.merged_by?.login,
        });

        const result = processMergeEvent(prEvent);
        console.log('Merge handler result:', result);

        if (result.shouldNotify && result.targetAgentId && result.message) {
          await injectToAgent(result.targetAgentId, result.message);
          console.log(`Notification sent to ${result.targetAgentId}`);
        }

        return {
          statusCode: 200,
          body: JSON.stringify({
            processed: true,
            eventType,
            action: prEvent.action,
            notification: result.shouldNotify ? {
              targetAgent: result.targetAgentId,
              sent: true,
            } : {
              sent: false,
              reason: result.reason,
            },
          }),
        };
      }

      case 'ping': {
        console.log('Ping event received - webhook configured successfully');
        return {
          statusCode: 200,
          body: JSON.stringify({ message: 'pong', processed: true }),
        };
      }

      default: {
        console.log(`Unhandled event type: ${eventType}`);
        return {
          statusCode: 200,
          body: JSON.stringify({ processed: false, reason: `Unhandled event type: ${eventType}` }),
        };
      }
    }
  } catch (error) {
    console.error('Error processing webhook:', error);
    return {
      statusCode: 500,
      body: JSON.stringify({ error: 'Internal server error', message: (error as Error).message }),
    };
  }
}

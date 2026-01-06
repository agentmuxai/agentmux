#!/usr/bin/env node

import { SecretsManagerClient, GetSecretValueCommand } from '@aws-sdk/client-secrets-manager';
import { AgentMuxServer } from './server.js';
import { Config } from './types.js';

/**
 * Load JWT secret from AWS Secrets Manager
 */
async function loadJwtSecret(): Promise<string> {
  const secretName = process.env.SECRET_NAME || 'services/infra';
  const region = process.env.AWS_REGION || 'us-east-1';

  const client = new SecretsManagerClient({ region });

  try {
    const response = await client.send(
      new GetSecretValueCommand({ SecretId: secretName })
    );

    if (!response.SecretString) {
      throw new Error('Secret has no string value');
    }

    const secret = JSON.parse(response.SecretString);
    const jwtSecret = secret['agentmux-jwt-secret'];

    if (!jwtSecret) {
      throw new Error(
        'agentmux-jwt-secret not found in services/infra secret. ' +
        'Add it manually: aws secretsmanager update-secret --secret-id services/infra ' +
        '--secret-string "$(jq \'.\\\"agentmux-jwt-secret\\\" = \\\"YOUR_SECRET\\"\' <<<\'$(aws secretsmanager get-secret-value --secret-id services/infra --query SecretString --output text)\')"'
      );
    }

    return jwtSecret;
  } catch (error) {
    console.error('Failed to load JWT secret from Secrets Manager:', error);
    throw error;
  }
}

/**
 * Main entry point
 */
async function main() {
  console.log('Starting AgentMux server...');

  // Load configuration
  const config: Config = {
    port: parseInt(process.env.PORT || '3100', 10),
    jwtSecret: await loadJwtSecret(),
    messagesTableName: process.env.MESSAGES_TABLE || 'agentmux-messages-prod',
    agentsTableName: process.env.AGENTS_TABLE || 'agentmux-agents-prod',
    region: process.env.AWS_REGION || 'us-east-1',
    secretName: process.env.SECRET_NAME || 'services/infra',
  };

  console.log('Configuration loaded:');
  console.log(`  Port: ${config.port}`);
  console.log(`  Messages table: ${config.messagesTableName}`);
  console.log(`  Agents table: ${config.agentsTableName}`);
  console.log(`  Region: ${config.region}`);

  // Start server
  const server = new AgentMuxServer(config);

  // Handle shutdown gracefully
  process.on('SIGTERM', () => {
    console.log('SIGTERM received, shutting down gracefully...');
    server.close();
    process.exit(0);
  });

  process.on('SIGINT', () => {
    console.log('SIGINT received, shutting down gracefully...');
    server.close();
    process.exit(0);
  });
}

main().catch((error) => {
  console.error('Fatal error:', error);
  process.exit(1);
});

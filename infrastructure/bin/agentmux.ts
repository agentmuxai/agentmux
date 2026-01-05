#!/usr/bin/env node
import 'source-map-support/register';
import * as cdk from 'aws-cdk-lib';
import { AgentMuxStack } from '../lib/agentmux-stack';

const app = new cdk.App();

new AgentMuxStack(app, 'agentmux-infrastructure', {
  environment: 'prod',
  env: {
    account: process.env.CDK_DEFAULT_ACCOUNT,
    region: process.env.CDK_DEFAULT_REGION || 'us-east-1',
  },
  description: 'AgentMux cloud infrastructure (WebSocket server on bastion)',
  tags: {
    Project: 'agentmux',
    Component: 'infrastructure',
    ManagedBy: 'CDK',
  },
});

app.synth();

import awsLambdaFastify from '@fastify/aws-lambda';
import { app } from './index.js';

// Create Lambda handler using Fastify adapter
export const handler = awsLambdaFastify(app);

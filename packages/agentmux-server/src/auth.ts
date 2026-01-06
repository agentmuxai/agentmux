import jwt from 'jsonwebtoken';

/**
 * Verify JWT token and extract agent ID
 */
export function verifyToken(token: string, secret: string): string | null {
  try {
    const decoded = jwt.verify(token, secret) as { agentId: string };
    return decoded.agentId;
  } catch (error) {
    console.error('JWT verification failed:', error);
    return null;
  }
}

/**
 * Generate JWT token for an agent (for testing/setup)
 */
export function generateToken(agentId: string, secret: string): string {
  return jwt.sign({ agentId }, secret, { expiresIn: '365d' });
}

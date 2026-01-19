/**
 * GitHub username to Agent ID mapping.
 *
 * Maps GitHub App bot usernames to internal agent IDs.
 * This mapping is used to determine which agent to notify
 * when a PR authored by that agent is merged.
 */

export const GITHUB_TO_AGENT_MAP: Record<string, string> = {
  'agentx-workflow[bot]': 'agentx',
  'agent1-workflow[bot]': 'agent1',
  'agent2-workflow[bot]': 'agent2',
  'agent3-workflow[bot]': 'agent3',
  'agent4-workflow[bot]': 'agent4',
  'agent5-workflow[bot]': 'agent5',
};

/**
 * Check if a GitHub username belongs to a known agent.
 */
export function isKnownAgent(githubUsername: string): boolean {
  return githubUsername in GITHUB_TO_AGENT_MAP;
}

/**
 * Get the agent ID for a GitHub username.
 * Returns undefined if not a known agent.
 */
export function getAgentId(githubUsername: string): string | undefined {
  return GITHUB_TO_AGENT_MAP[githubUsername];
}

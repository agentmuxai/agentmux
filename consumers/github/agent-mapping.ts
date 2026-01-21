/**
 * GitHub username to Agent ID mapping.
 *
 * Maps GitHub identities to internal agent IDs. Agents use two identity patterns:
 * - GitHub App (MCP): agent{x|y|a-g|1-5}-workflow[bot]
 * - PAT (gh CLI): Agent{X|Y|A-G|1-5}-asaf
 *
 * This mapping is used to determine which agent to notify
 * when a PR authored by that agent receives events.
 */

// Static mapping for GitHub App bot usernames (exact match)
export const GITHUB_TO_AGENT_MAP: Record<string, string> = {
  'agentx-workflow[bot]': 'agentx',
  'agenty-workflow[bot]': 'agenty',
  'agenta-workflow[bot]': 'agenta',
  'agentb-workflow[bot]': 'agentb',
  'agentc-workflow[bot]': 'agentc',
  'agentd-workflow[bot]': 'agentd',
  'agente-workflow[bot]': 'agente',
  'agentf-workflow[bot]': 'agentf',
  'agentg-workflow[bot]': 'agentg',
  'agent1-workflow[bot]': 'agent1',
  'agent2-workflow[bot]': 'agent2',
  'agent3-workflow[bot]': 'agent3',
  'agent4-workflow[bot]': 'agent4',
  'agent5-workflow[bot]': 'agent5',
};

// Regex patterns for dynamic matching (lowercase - input is normalized before matching)
const GITHUB_APP_PATTERN = /^agent([xya-g]|\d)-workflow\[bot\]$/;
const PAT_PATTERN = /^agent([xya-g]|\d)-asaf$/;

/**
 * Extract agent ID from GitHub username using pattern matching.
 * Returns undefined if not a known agent pattern.
 *
 * All lookups are case-insensitive: input is lowercased before matching
 * against the static map (which has lowercase keys) and regex patterns.
 *
 * Examples:
 *   "agentx-workflow[bot]" → "agentx"
 *   "AgentA-asaf" → "agenta"
 *   "agent1-workflow[bot]" → "agent1"
 *   "Agent4-asaf" → "agent4"
 *   "reagentx-workflow[bot]" → undefined (ReAgent, not an agent)
 *   "a5af" → undefined
 */
export function getAgentId(githubUsername: string): string | undefined {
  if (!githubUsername) return undefined;

  // Normalize to lowercase for case-insensitive matching
  const lowerUsername = githubUsername.toLowerCase();

  // Check static map first (map keys are lowercase to match normalized input)
  if (lowerUsername in GITHUB_TO_AGENT_MAP) {
    return GITHUB_TO_AGENT_MAP[lowerUsername];
  }

  // Try GitHub App pattern: agent{x|y|a-g|1-5}-workflow[bot]
  const appMatch = lowerUsername.match(GITHUB_APP_PATTERN);
  if (appMatch) {
    return `agent${appMatch[1]}`;
  }

  // Try PAT pattern: agent{x|y|a-g|1-5}-asaf
  const patMatch = lowerUsername.match(PAT_PATTERN);
  if (patMatch) {
    return `agent${patMatch[1]}`;
  }

  return undefined;
}

/**
 * Check if a GitHub username belongs to a known agent.
 */
export function isKnownAgent(githubUsername: string): boolean {
  return getAgentId(githubUsername) !== undefined;
}

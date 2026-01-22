import type { Session } from '../types';

export function supportsPlanMode(agentType?: Session['agent_type'] | null): boolean {
  return agentType === 'claude' || agentType === 'codex' || agentType === 'gemini';
}

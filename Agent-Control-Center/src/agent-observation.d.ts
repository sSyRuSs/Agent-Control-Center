export type ObservedAgent = { status: string; startedAt: string; updatedAt: string; command: string; currentFile: string; detail: string };
export function validTime(value: string): number | null;
export function elapsedLabel(seconds: number): string;
export function lifecycleLabel(agent: Pick<ObservedAgent, 'status'|'startedAt'|'updatedAt'>, now?: number): string;
export function latestEvidence(agent: Pick<ObservedAgent, 'updatedAt'|'command'|'currentFile'|'detail'>): { label: string; text: string; source: string };

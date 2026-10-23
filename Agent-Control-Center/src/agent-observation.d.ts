export type ObservedAgent = { status: string; statusObservedAt?: string; statusSource?: string; startedAt: string; updatedAt: string; command: string; currentFile: string; detail: string };
export const STATUS_FRESHNESS_MS: number;
export function validTime(value: string): number | null;
export function elapsedLabel(seconds: number): string;
export function statusObservation(agent: Pick<ObservedAgent, 'statusObservedAt'>, now?: number): { freshness: 'recent'|'stale'|'unknown'; ageMs: number|null; observedAt: number|null };
export function statusPresentation(agent: Pick<ObservedAgent, 'status'|'statusObservedAt'>, now?: number): { freshness: 'recent'|'stale'|'unknown'; ageMs: number|null; observedAt: number|null; label: string; badgeClass: string; isRecentlyWorking: boolean };
export function lifecycleLabel(agent: Pick<ObservedAgent, 'status'|'statusObservedAt'|'startedAt'|'updatedAt'>, now?: number): string;
export function latestEvidence(agent: Pick<ObservedAgent, 'updatedAt'|'command'|'currentFile'|'detail'>): { label: string; text: string; source: string };

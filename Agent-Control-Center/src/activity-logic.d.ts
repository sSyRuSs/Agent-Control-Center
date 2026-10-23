export type ActivityRecord = { id: number };
export function mergeActivity<T extends ActivityRecord>(existing: T[], incoming: T[]): T[];
export function shouldAcceptActivityResponse(requestGeneration: number, currentGeneration: number): boolean;
export function prepareForegroundActivityRequest(): { loading: boolean; loadingOlder: boolean; refreshing: boolean };
export type ActivityEvent = ActivityRecord & { at: string; role: string; kind: string; text: string; threadId?: string };
export type ActivityGroup<T extends ActivityEvent = ActivityEvent> = { key: string; id: number; role: string; kind: string; threadId?: string; newestAt: string; oldestAt: string; text: string; events: T[] };
export function activityKindLabel(kind: string): string;
export function groupAdjacentActivity<T extends ActivityEvent>(events: T[]): ActivityGroup<T>[];

export type ActivityRecord = { id: number };
export function mergeActivity<T extends ActivityRecord>(existing: T[], incoming: T[]): T[];
export function shouldAcceptActivityResponse(requestGeneration: number, currentGeneration: number): boolean;
export function prepareForegroundActivityRequest(): { loading: boolean; loadingOlder: boolean; refreshing: boolean };

const terminal = new Set(['done', 'failed', 'blocked']);
export const STATUS_FRESHNESS_MS = 120_000;

export function validTime(value) {
  const time = value ? new Date(value).getTime() : Number.NaN;
  return Number.isFinite(time) ? time : null;
}

export function elapsedLabel(seconds) {
  const safe = Math.max(0, Math.floor(seconds));
  if (safe < 60) return `${safe}s`;
  if (safe < 3600) return `${Math.floor(safe / 60)}m ${safe % 60}s`;
  return `${Math.floor(safe / 3600)}h ${Math.floor((safe % 3600) / 60)}m`;
}

export function statusObservation(agent, now = Date.now()) {
  const observedAt = validTime(agent.statusObservedAt);
  if (observedAt === null || observedAt > now) return { freshness: 'unknown', ageMs: null, observedAt };
  const ageMs = now - observedAt;
  return { freshness: ageMs <= STATUS_FRESHNESS_MS ? 'recent' : 'stale', ageMs, observedAt };
}

export function statusPresentation(agent, now = Date.now()) {
  const observation = statusObservation(agent, now);
  const status = agent.status || 'idle';
  const name = status.charAt(0).toUpperCase() + status.slice(1);
  if (observation.freshness === 'recent') {
    return { ...observation, label: `${name} · recent`, badgeClass: status, isRecentlyWorking: status === 'working' };
  }
  if (observation.freshness === 'stale') {
    return { ...observation, label: `${name} · last observed`, badgeClass: 'stale', isRecentlyWorking: false };
  }
  return { ...observation, label: `${name} · time unknown`, badgeClass: 'unknown', isRecentlyWorking: false };
}

export function lifecycleLabel(agent, now = Date.now()) {
  const start = validTime(agent.startedAt);
  const updated = validTime(agent.updatedAt);
  const observation = statusObservation(agent, now);
  if (agent.status === 'working' && observation.freshness === 'recent') return start === null ? 'Recently observed working; start unavailable' : `Observed runtime ${elapsedLabel((now - start) / 1000)}`;
  if (agent.status === 'working' && observation.freshness === 'stale') return `Last observed working ${elapsedLabel(observation.ageMs / 1000)} ago`;
  if (agent.status === 'working') return 'Working was observed; observation time unavailable';
  if (terminal.has(agent.status) && start !== null && observation.observedAt !== null && observation.observedAt >= start) return `Observed duration ${elapsedLabel((observation.observedAt - start) / 1000)}`;
  if (terminal.has(agent.status)) return 'Terminal duration unavailable';
  if (updated !== null) return `Observed ${elapsedLabel((now - updated) / 1000)} ago`;
  return 'No observation time';
}

export function latestEvidence(agent) {
  if (agent.command) return { label: 'LATEST COMMAND', text: agent.command, source: 'command' };
  if (agent.currentFile) return { label: 'LATEST FILE', text: agent.currentFile, source: 'file' };
  if (agent.detail) return { label: agent.detail === 'Dispatched · status unavailable' ? 'DISPATCH EVIDENCE' : 'LATEST EVIDENCE', text: agent.detail, source: 'detail' };
  if (!validTime(agent.updatedAt)) return { label: 'LATEST EVIDENCE', text: 'No session observed', source: 'none' };
  return { label: 'LATEST EVIDENCE', text: 'No lifecycle evidence observed', source: 'none' };
}

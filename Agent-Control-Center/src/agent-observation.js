const terminal = new Set(['done', 'failed', 'blocked']);

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

export function lifecycleLabel(agent, now = Date.now()) {
  const start = validTime(agent.startedAt);
  const updated = validTime(agent.updatedAt);
  if (agent.status === 'working') return start === null ? 'Observed working; start unavailable' : `Working since observed start ${elapsedLabel((now - start) / 1000)}`;
  if (terminal.has(agent.status) && start !== null && updated !== null) return `Observed duration ${elapsedLabel((updated - start) / 1000)}`;
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

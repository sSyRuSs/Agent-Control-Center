export function mergeActivity(existing, incoming) {
  return Array.from(new Map([...incoming, ...existing].map((item) => [item.id, item])).values())
    .sort((left, right) => right.id - left.id);
}

export function shouldAcceptActivityResponse(requestGeneration, currentGeneration) {
  return requestGeneration === currentGeneration;
}

export function prepareForegroundActivityRequest() {
  return { loading: true, loadingOlder: false, refreshing: false };
}

const kindLabels = {
  session: 'Session',
  status: 'Status',
  turn: 'Model turn',
  item: 'Work item',
  tool: 'Tool call',
  command: 'Command',
  file: 'File change',
  message: 'Agent message',
  delegation: 'Delegation',
  approval: 'Approval',
  error: 'Error',
  appServer: 'App Server',
};

export function activityKindLabel(kind) {
  if (!kind) return 'Event';
  return kindLabels[kind] || kind
    .replace(/([a-z0-9])([A-Z])/g, '$1 $2')
    .replace(/[\/_-]+/g, ' ')
    .replace(/^./, (letter) => letter.toUpperCase());
}

export function groupAdjacentActivity(events) {
  return events.reduce((groups, event) => {
    const previous = groups[groups.length - 1];
    const previousTime = previous ? new Date(previous.oldestAt).getTime() : Number.NaN;
    const eventTime = new Date(event.at).getTime();
    const withinBurst = Number.isFinite(previousTime) && Number.isFinite(eventTime)
      && previousTime >= eventTime && previousTime - eventTime <= 30_000;
    const critical = new Set(['session', 'status', 'turn', 'approval', 'error']);
    const sameStream = previous
      && previous.role === event.role
      && previous.kind === event.kind
      && Boolean(previous.threadId)
      && previous.threadId === event.threadId
      && withinBurst
      && (!critical.has(event.kind) || previous.text === event.text);
    if (sameStream) {
      previous.events.push(event);
      previous.oldestAt = event.at;
      return groups;
    }
    groups.push({
      key: String(event.id),
      id: event.id,
      role: event.role,
      kind: event.kind,
      threadId: event.threadId,
      newestAt: event.at,
      oldestAt: event.at,
      text: event.text,
      events: [event],
    });
    return groups;
  }, []);
}

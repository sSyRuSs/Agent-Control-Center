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

import test from 'node:test';
import assert from 'node:assert/strict';
import { latestEvidence, lifecycleLabel } from '../src/agent-observation.js';

test('only working status receives a ticking runtime label', () => {
  const now = Date.parse('2026-10-08T00:01:00Z');
  assert.equal(lifecycleLabel({ status: 'working', startedAt: '2026-10-08T00:00:00Z', updatedAt: '' }, now), 'Working since observed start 1m 0s');
  assert.equal(lifecycleLabel({ status: 'waiting', startedAt: '2026-10-08T00:00:00Z', updatedAt: '2026-10-08T00:00:20Z' }, now), 'Observed 40s ago');
  assert.equal(lifecycleLabel({ status: 'done', startedAt: '2026-10-08T00:00:00Z', updatedAt: '2026-10-08T00:00:20Z' }, now), 'Observed duration 20s');
});

test('missing or invalid timestamps do not fabricate session evidence', () => {
  assert.equal(lifecycleLabel({ status: 'idle', startedAt: '', updatedAt: 'bad timestamp' }), 'No observation time');
  assert.deepEqual(latestEvidence({ command: '', currentFile: '', detail: '', updatedAt: '' }), { label: 'LATEST EVIDENCE', text: 'No session observed', source: 'none' });
  assert.equal(latestEvidence({ command: '', currentFile: '', detail: 'Dispatched · status unavailable', updatedAt: '' }).label, 'DISPATCH EVIDENCE');
});

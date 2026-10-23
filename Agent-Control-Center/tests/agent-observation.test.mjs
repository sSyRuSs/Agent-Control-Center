import test from 'node:test';
import assert from 'node:assert/strict';
import { latestEvidence, lifecycleLabel, statusObservation, statusPresentation } from '../src/agent-observation.js';

test('only working status receives a ticking runtime label', () => {
  const now = Date.parse('2026-10-08T00:01:00Z');
  assert.equal(lifecycleLabel({ status: 'working', statusObservedAt: '2026-10-08T00:00:50Z', startedAt: '2026-10-08T00:00:00Z', updatedAt: '' }, now), 'Observed runtime 1m 0s');
  assert.equal(lifecycleLabel({ status: 'waiting', startedAt: '2026-10-08T00:00:00Z', updatedAt: '2026-10-08T00:00:20Z' }, now), 'Observed 40s ago');
  assert.equal(lifecycleLabel({ status: 'done', statusObservedAt: '2026-10-08T00:00:20Z', startedAt: '2026-10-08T00:00:00Z', updatedAt: '2026-10-08T00:00:20Z' }, now), 'Observed duration 20s');
});

test('terminal duration ends at lifecycle evidence instead of later activity', () => {
  const now = Date.parse('2026-10-08T00:06:00Z');
  assert.equal(lifecycleLabel({
    status: 'done',
    statusObservedAt: '2026-10-08T00:00:20Z',
    startedAt: '2026-10-08T00:00:00Z',
    updatedAt: '2026-10-08T00:05:00Z',
  }, now), 'Observed duration 20s');
  assert.equal(lifecycleLabel({
    status: 'done',
    statusObservedAt: '',
    startedAt: '2026-10-08T00:00:00Z',
    updatedAt: '2026-10-08T00:05:00Z',
  }, now), 'Terminal duration unavailable');
});

test('stale working state is historical and does not claim a live runtime', () => {
  const now = Date.parse('2026-10-08T00:05:00Z');
  const agent = { status: 'working', statusObservedAt: '2026-10-08T00:01:00Z', startedAt: '2026-10-08T00:00:00Z', updatedAt: '2026-10-08T00:01:00Z' };
  assert.equal(statusPresentation(agent, now).isRecentlyWorking, false);
  assert.equal(statusPresentation(agent, now).label, 'Working · last observed');
  assert.equal(lifecycleLabel(agent, now), 'Last observed working 4m 0s ago');
});

test('missing, invalid, and future status timestamps have unknown freshness', () => {
  const now = Date.parse('2026-10-08T00:05:00Z');
  assert.equal(statusObservation({ statusObservedAt: '' }, now).freshness, 'unknown');
  assert.equal(statusObservation({ statusObservedAt: 'bad' }, now).freshness, 'unknown');
  assert.equal(statusObservation({ statusObservedAt: '2026-10-08T00:05:01Z' }, now).freshness, 'unknown');
  assert.equal(statusPresentation({ status: 'done', statusObservedAt: '2026-10-08T00:04:30Z' }, now).label, 'Done · recent');
});

test('missing or invalid timestamps do not fabricate session evidence', () => {
  assert.equal(lifecycleLabel({ status: 'idle', startedAt: '', updatedAt: 'bad timestamp' }), 'No observation time');
  assert.deepEqual(latestEvidence({ command: '', currentFile: '', detail: '', updatedAt: '' }), { label: 'LATEST EVIDENCE', text: 'No session observed', source: 'none' });
  assert.equal(latestEvidence({ command: '', currentFile: '', detail: 'Dispatched · status unavailable', updatedAt: '' }).label, 'DISPATCH EVIDENCE');
});

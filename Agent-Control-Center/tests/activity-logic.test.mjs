import test from 'node:test';
import assert from 'node:assert/strict';
import { activityKindLabel, groupAdjacentActivity, mergeActivity, prepareForegroundActivityRequest, shouldAcceptActivityResponse } from '../src/activity-logic.js';

test('a filter generation rejects a delayed response from an older filter', () => {
  const delayedRequest = 4;
  const generationAfterFilterChange = 5;
  assert.equal(shouldAcceptActivityResponse(delayedRequest, generationAfterFilterChange), false);
  assert.equal(shouldAcceptActivityResponse(generationAfterFilterChange, generationAfterFilterChange), true);
});

test('loading older history merges without duplicate event IDs and keeps newest record first', () => {
  const newest = [{ id: 12, text: 'new' }, { id: 11, text: 'current' }];
  const older = [{ id: 11, text: 'duplicate' }, { id: 10, text: 'old' }];
  assert.deepEqual(mergeActivity(newest, older), [{ id: 12, text: 'new' }, { id: 11, text: 'current' }, { id: 10, text: 'old' }]);
});

test('a foreground filter request clears an invalidated background refresh state', () => {
  assert.deepEqual(prepareForegroundActivityRequest(), { loading: true, loadingOlder: false, refreshing: false });
});

test('adjacent events from the same role, thread and kind form an inspectable group', () => {
  const events = [
    { id: 5, at: '2026-10-19T01:00:05Z', role: 'Coder', threadId: 'a', kind: 'message', text: 'newest' },
    { id: 4, at: '2026-10-19T01:00:04Z', role: 'Coder', threadId: 'a', kind: 'message', text: 'older' },
    { id: 3, at: '2026-10-19T01:00:03Z', role: 'Coder', threadId: 'b', kind: 'message', text: 'other thread' },
    { id: 2, at: '2026-10-19T01:00:02Z', role: 'Tester', threadId: 'b', kind: 'error', text: 'failed' },
  ];
  const groups = groupAdjacentActivity(events);
  assert.equal(groups.length, 3);
  assert.deepEqual(groups[0].events.map(event => event.id), [5, 4]);
  assert.equal(groups[0].newestAt, events[0].at);
  assert.equal(groups[0].oldestAt, events[1].at);
  assert.equal(groups[1].events.length, 1);
});

test('event kinds have readable labels with a safe fallback', () => {
  assert.equal(activityKindLabel('appServer'), 'App Server');
  assert.equal(activityKindLabel('file_change'), 'File change');
  assert.equal(activityKindLabel(''), 'Event');
});

test('groups do not merge unknown streams, distant events, or distinct critical milestones', () => {
  const base = { role: 'Planner', kind: 'message', text: 'delta' };
  const events = [
    { ...base, id: 8, at: '2026-10-19T01:02:00Z' },
    { ...base, id: 7, at: '2026-10-19T01:01:59Z' },
    { ...base, id: 6, at: '2026-10-19T01:01:58Z', threadId: 'a' },
    { ...base, id: 5, at: '2026-10-19T01:00:00Z', threadId: 'a' },
    { id: 4, at: '2026-10-19T00:59:59Z', role: 'Planner', threadId: 'a', kind: 'status', text: 'working' },
    { id: 3, at: '2026-10-19T00:59:58Z', role: 'Planner', threadId: 'a', kind: 'status', text: 'done' },
  ];
  assert.equal(groupAdjacentActivity(events).length, events.length);
});

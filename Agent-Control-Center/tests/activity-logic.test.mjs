import test from 'node:test';
import assert from 'node:assert/strict';
import { mergeActivity, prepareForegroundActivityRequest, shouldAcceptActivityResponse } from '../src/activity-logic.js';

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

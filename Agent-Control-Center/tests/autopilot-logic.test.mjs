import test from 'node:test';
import assert from 'node:assert/strict';
import { allowedPlanActions, buildGateInvocation, buildQuestionAnswers, createSoftwareTemplate, humanRequestIdentity, humanRequestKind, parseDependencies, structuredGateActions, validateAutopilotInput } from '../src/autopilot-logic.js';

const valid = () => ({
  title: 'Feature', objective: 'Build feature', workspace: 'C:\\repo',
  limits: { maxAttempts: 2, maxRuntimeMinutes: 90, maxTokens: 100000, maxParallel: 1 },
  tasks: createSoftwareTemplate('Build feature'),
});

test('software template includes implementation, verification, independent review and a human gate', () => {
  const tasks = createSoftwareTemplate('Add autopilot');
  assert.deepEqual(tasks.map(task => task.role), ['planner', 'coder', 'tester', 'reviewer', 'human']);
  assert.deepEqual(tasks.at(-1).dependencies, ['review']);
  assert.deepEqual(validateAutopilotInput(valid()), []);
});

test('dependency parsing trims values and removes duplicates', () => {
  assert.deepEqual(parseDependencies('plan, implement, plan, '), ['plan', 'implement']);
});

test('validation rejects missing references and cyclic dependency maps', () => {
  const missing = valid();
  missing.tasks[1].dependencies = ['unknown'];
  assert.ok(validateAutopilotInput(missing).some(message => message.includes('missing task unknown')));
  const cyclic = valid();
  cyclic.tasks[0].dependencies = ['human-review'];
  assert.ok(validateAutopilotInput(cyclic).some(message => message.includes('cycle')));
});

test('control state requires explicit approval of the current revision', () => {
  assert.equal(allowedPlanActions({ status: 'approved', revision: 3, approvedRevision: 2 }).start, false);
  assert.equal(allowedPlanActions({ status: 'approved', revision: 3, approvedRevision: 3 }).start, true);
  assert.equal(allowedPlanActions({ status: 'running', revision: 3 }).pause, true);
  assert.equal(allowedPlanActions({ status: 'needsInput', revision: 3 }).cancel, true);
});

test('human request methods are classified explicitly and unknown methods stay blocked', () => {
  assert.equal(humanRequestKind('item/commandExecution/requestApproval'), 'command');
  assert.equal(humanRequestKind('item/fileChange/requestApproval'), 'file');
  assert.equal(humanRequestKind('item/permissions/requestApproval'), 'permissions');
  assert.equal(humanRequestKind('item/tool/requestUserInput'), 'questions');
  assert.equal(humanRequestKind('mcpServer/elicitation/request'), 'elicitation');
  assert.equal(humanRequestKind('future/unknown/request'), 'unsupported');
});

test('typed question answers use the app-server answer object and omit blanks', () => {
  const questions = [{ id: 'target' }, { id: 'notes' }];
  assert.deepEqual(buildQuestionAnswers(questions, { target: 'Windows', notes: '  ' }), {
    target: { answers: ['Windows'] },
  });
});

test('human request identity is stable across queue reorder and separates stale turns', () => {
  const first = { requestId: { sequence: 4, source: 'codex' }, threadId: 'thread-a', turnId: 'turn-1' };
  const second = { requestId: 9, threadId: 'thread-a', turnId: 'turn-1' };
  const before = [first, second].map(request => humanRequestIdentity(3, 'code', request));
  const after = [second, first].map(request => humanRequestIdentity(3, 'code', request));
  assert.equal(before[0], after[1]);
  assert.equal(before[1], after[0]);
  assert.notEqual(humanRequestIdentity(3, 'code', first), humanRequestIdentity(3, 'code', { ...first, turnId: 'turn-2' }));
  assert.notEqual(humanRequestIdentity(3, 'code', first), humanRequestIdentity(4, 'code', first));
});

test('cancelling is a locked transitional plan state', () => {
  const actions = allowedPlanActions({ status: 'cancelling', revision: 4, approvedRevision: 4 });
  assert.deepEqual(actions, { save: false, approve: false, start: false, pause: false, resume: false, cancel: false });
});

test('nonhuman gate labels dispatch distinct continue and terminal reject IPC payloads', () => {
  const actions = structuredGateActions('coder');
  assert.deepEqual(actions.map(item => item.label), ['Continue with direction', 'Reject and end run']);
  const continueAction = actions.find(item => item.label === 'Continue with direction');
  const rejectAction = actions.find(item => item.label === 'Reject and end run');
  assert.deepEqual(buildGateInvocation('plan-1', 'code', 'fix the edge case', continueAction.action), {
    command: 'respond_autopilot_gate',
    args: { planId: 'plan-1', taskId: 'code', response: 'fix the edge case', approve: false, retry: true },
  });
  assert.deepEqual(buildGateInvocation('plan-1', 'code', 'stop here', rejectAction.action), {
    command: 'respond_autopilot_gate',
    args: { planId: 'plan-1', taskId: 'code', response: 'stop here', approve: false, retry: false },
  });
});

test('human final gate keeps approve and retry separate from rejection', () => {
  const actions = structuredGateActions('human');
  assert.deepEqual(actions.map(item => item.action), ['approve', 'retry', 'reject']);
  assert.deepEqual(buildGateInvocation('plan-1', 'human-review', 'accepted', actions[0].action).args, {
    planId: 'plan-1', taskId: 'human-review', response: 'accepted', approve: true, retry: false,
  });
});

export const taskRoles = ['planner', 'coder', 'mobile-dev', 'tester', 'reviewer', 'human'];

export function createSoftwareTemplate(objective = '') {
  const work = objective.trim() || 'Implement the approved requirement';
  return [
    { id: 'plan', title: 'Plan implementation', description: `Turn this requirement into an implementation plan: ${work}`, role: 'planner', dependencies: [] },
    { id: 'implement', title: 'Implement changes', description: work, role: 'coder', dependencies: ['plan'] },
    { id: 'test', title: 'Verify implementation', description: 'Run the relevant checks and report evidence.', role: 'tester', dependencies: ['implement'] },
    { id: 'review', title: 'Independent review', description: 'Review correctness, regressions, security risks, and missing tests.', role: 'reviewer', dependencies: ['test'] },
    { id: 'human-review', title: 'Human final review', description: 'Review the completed work and decide whether to accept it.', role: 'human', dependencies: ['review'] },
  ];
}

export function parseDependencies(value) {
  return [...new Set(String(value).split(',').map(item => item.trim()).filter(Boolean))];
}

export function validateAutopilotInput(input) {
  const errors = [];
  if (!input.title?.trim()) errors.push('Plan title is required.');
  if (!input.objective?.trim()) errors.push('Requirement is required.');
  if (!input.workspace?.trim()) errors.push('Workspace path is required.');
  if (!input.tasks?.length) errors.push('Add at least one task.');
  const ids = new Set();
  for (const task of input.tasks || []) {
    if (!task.id?.trim()) errors.push('Every task needs an ID.');
    else if (ids.has(task.id)) errors.push(`Duplicate task ID: ${task.id}.`);
    else ids.add(task.id);
    if (!task.title?.trim()) errors.push(`Task ${task.id || '(untitled)'} needs a title.`);
    if (!task.description?.trim()) errors.push(`Task ${task.id || '(untitled)'} needs instructions.`);
    if (!taskRoles.includes(task.role)) errors.push(`Task ${task.id || '(untitled)'} has an unsupported role.`);
  }
  for (const task of input.tasks || []) {
    for (const dependency of task.dependencies || []) {
      if (dependency === task.id) errors.push(`Task ${task.id} cannot depend on itself.`);
      else if (!ids.has(dependency)) errors.push(`Task ${task.id} depends on missing task ${dependency}.`);
    }
  }
  const visiting = new Set();
  const visited = new Set();
  const byId = new Map((input.tasks || []).map(task => [task.id, task]));
  function visit(id) {
    if (visiting.has(id)) return true;
    if (visited.has(id)) return false;
    visiting.add(id);
    const cyclic = (byId.get(id)?.dependencies || []).some(dependency => byId.has(dependency) && visit(dependency));
    visiting.delete(id);
    visited.add(id);
    return cyclic;
  }
  if ([...byId.keys()].some(visit)) errors.push('Dependency map contains a cycle.');
  const limits = input.limits || {};
  if (!Number.isInteger(limits.maxAttempts) || limits.maxAttempts < 1 || limits.maxAttempts > 5) errors.push('Attempts must be between 1 and 5.');
  if (!Number.isInteger(limits.maxRuntimeMinutes) || limits.maxRuntimeMinutes < 1) errors.push('Runtime limit must be at least 1 minute.');
  if (!Number.isInteger(limits.maxTokens) || limits.maxTokens < 1) errors.push('Token limit must be positive.');
  if (limits.maxParallel !== 1) errors.push('V2 currently supports serial execution only (max parallel = 1).');
  return [...new Set(errors)];
}

export function allowedPlanActions(plan, busy = false) {
  const status = plan?.status;
  return {
    save: !busy && !['running', 'paused', 'cancelling'].includes(status),
    approve: !busy && status === 'draft',
    start: !busy && status === 'approved' && plan.approvedRevision === plan.revision,
    pause: !busy && status === 'running',
    resume: !busy && status === 'paused',
    cancel: !busy && ['running', 'paused', 'needsInput'].includes(status),
  };
}

export function runtimeSeconds(task, nowSeconds = Date.now() / 1000) {
  if (!task?.startedAt) return 0;
  return Math.max(0, Math.floor((task.finishedAt || nowSeconds) - task.startedAt));
}

export function formatDuration(seconds) {
  const total = Math.max(0, Math.floor(seconds || 0));
  const hours = Math.floor(total / 3600);
  const minutes = Math.floor((total % 3600) / 60);
  const secs = total % 60;
  return hours ? `${hours}h ${minutes}m` : minutes ? `${minutes}m ${secs}s` : `${secs}s`;
}

export function humanRequestKind(method) {
  if (method === 'item/commandExecution/requestApproval') return 'command';
  if (method === 'item/fileChange/requestApproval') return 'file';
  if (method === 'item/permissions/requestApproval') return 'permissions';
  if (method === 'item/tool/requestUserInput') return 'questions';
  if (method === 'mcpServer/elicitation/request') return 'elicitation';
  return 'unsupported';
}

export function buildQuestionAnswers(questions, values) {
  const answers = {};
  for (const question of questions || []) {
    const id = String(question?.id || '').trim();
    const answer = String(values?.[id] || '').trim();
    if (id && answer) answers[id] = { answers: [answer] };
  }
  return answers;
}

function canonicalize(value) {
  if (Array.isArray(value)) return value.map(canonicalize);
  if (value && typeof value === 'object') {
    return Object.fromEntries(Object.keys(value).sort().map(key => [key, canonicalize(value[key])]));
  }
  return value;
}

export function humanRequestIdentity(planRevision, taskId, request) {
  return JSON.stringify(canonicalize({
    planRevision,
    taskId,
    requestId: request?.requestId,
    threadId: request?.threadId || '',
    turnId: request?.turnId || '',
  }));
}

export function structuredGateActions(role) {
  return role === 'human'
    ? [
        { action: 'approve', label: 'Approve final gate' },
        { action: 'retry', label: 'Retry with notes' },
        { action: 'reject', label: 'Reject and end run' },
      ]
    : [
        { action: 'continue', label: 'Continue with direction' },
        { action: 'reject', label: 'Reject and end run' },
      ];
}

export function buildGateInvocation(planId, taskId, response, action) {
  const decision = {
    approve: { approve: true, retry: false },
    retry: { approve: false, retry: true },
    continue: { approve: false, retry: true },
    reject: { approve: false, retry: false },
  }[action];
  if (!decision) throw new Error(`Unsupported gate action: ${action}`);
  return {
    command: 'respond_autopilot_gate',
    args: { planId, taskId, response, ...decision },
  };
}

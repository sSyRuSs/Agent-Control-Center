# Native and Autopilot smoke report — 2026-10-19

App: Agent Control Center 0.2.2 Windows EXE. Fixture: ../Autopilot-Smoke-20261018.

## Verified

- Native EXE reads real Codex monitor events. Light mode and sidebar collapse work and persist after closing/reopening the app.
- UI-created revision 4 was saved, explicitly approved, and started. Real Codex App Server executed each role sequentially in separate threads, honoring dependencies.
- Planner → Coder → Tester → Reviewer all passed; controller stopped at human-review / needsInput. The user explicitly approved on 2026-10-19; the final gate was submitted through the native UI. Persisted plan status is completed, with all 5 tasks passed.
- Runtime from first task start to reviewer completion: 92 seconds. Reported tokens: 83,620 input + 1,372 output = 84,992. These are reported tokens, not monetary cost.
- Tester ran npm test, 7/7 passed. Reviewer independently approved implementation, noting no explicit string/null input tests. Parent reran the same suite: 7/7 passed.

| Role | Model | Thread |
| --- | --- | --- |
| Planner | gpt-6-astra | 01a1528c-e36e-7860-a33d-d80ff0b8b818 |
| Coder | gpt-5.6-sol | 01a1528d-2167-7172-aced-7b723cb1ebf4 |
| Tester | gpt-5.6-luna | 01a1528d-abb0-7f23-badc-2a9d99cb1fe8 |
| Reviewer | gpt-5.6-terra | 01a1528d-d9d0-7c63-bc8c-d3bf91931523 |

## Prior revision 3 and recovery

The first real run created clamp.js and an independent clamp.test.js. A deliberate Planner clarification exercised needsInput → Continue with direction → running → passed. The button did not reject the plan.

Default node:test process isolation encountered EPERM in the execution sandbox. The Tester reported failure despite a successful no-isolation rerun; controller routed it to implementation repair. The run then stopped at its 100K observed-token ceiling (104,961 reported tokens after an update). This verifies bounded stopping, not a hard billing limit. Evidence is saved in revision3-token-limit.json.

Only the fixture package.json test command was changed to `node --test --test-isolation=none`; sandbox policy was not weakened. Revision 4 reused the real implementation/tests, omitted the deliberate clarification, and verified the full role chain. Restart recovered revision 3 as interrupted rather than silently continuing it.

## Remaining limitations / findings

- Launch through the native automation helper initially failed to spawn Codex App Server. Relaunch with C:/nodejs first in process-only PATH succeeded with npm Codex CLI 0.153.4 and 4 models. Global PATH/config was not changed. Root cause of the helper launch failure is not conclusively established; executable discovery needs a separate follow-up.
- Overview contains stale subagent observations; it should not be treated as authoritative live task status. Managed Autopilot thread/status evidence was used for this run.
- Managed event history is limited to 500 events and delta notifications crowd out older lifecycle events. Saved snapshots preserve this run's outcome; better event grouping remains follow-up work.
- Native light/dark and persistence were checked; arbitrary native window resize was not repeated here. Prior browser checks covered 960×640 and 1440×900.
- Test coverage does not explicitly exercise non-number strings/null. The implementation uses Number.isFinite and Reviewer approved with that note.

## State preservation

The original user's failed E:/Eat-clean-web plan was backed up as autopilot-before-*.json in this folder before the smoke plan was saved. No work was performed in that project. Current live state is revision 4 completed after explicit user approval. Snapshots: revision4-human-review.json (before approval), revision4-completed.json (after approval). Event 2422 records Human gate approved. Token usage stayed unchanged after approval; no further model turn was dispatched.

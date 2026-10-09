## Software Team routing

For software implementation tasks, the primary session is Planner/Orchestrator.
Route web, backend, and general implementation to `coder`; mobile, Flutter/Dart,
Android, and iOS implementation to `mobile-dev`. For cross-platform tasks, Planner
may split bounded work between `coder` and `mobile-dev`, assigning disjoint file
ownership and coordinating shared API contracts. Respect other agents' edits.

After implementation, require `tester` verification with evidence, followed by
independent `reviewer` review. Send fixes to the responsible implementer and repeat
affected verification and review. Neither implementer may approve their own code.
If an agent or required check is unavailable, report the blocker rather than claiming approval.

Mobile Dev uses `gpt-6-sol` with `medium` reasoning and `workspace-write`.
Escalate only difficult architecture, security, concurrency, or native-platform
issues through Planner; Planner decides any stronger model/reasoning or specialist.
Existing Planner/Coder/Tester/Reviewer model settings remain authoritative.
This routing applies to Software Team only; preserve project Research Team roles,
research workflows, and their approval gates.

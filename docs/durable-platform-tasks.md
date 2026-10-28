# Durable Platform tasks

For a Dev account, open **Tasks** from Fleet or the remote Agents detail. A local
project must have an exact Platform v2 project/user-workspace mapping. The daemon
and gateway must support the task protocol and authorize the workspace's task
execution grant; a client cannot grant itself execution rights.

Draft a task objective and one or more artifact checks. Add the task to the plan,
then add dependent tasks by selecting earlier draft tasks. Set attempts, per-attempt
minutes, plan concurrency and total minutes. Review the full scope and plan in the
confirmation dialog before creating it.

Execution currently occurs in private per-attempt workspaces. Tasks do not edit
the mapped project checkout. Dependencies gate execution on verified predecessors.
The server can hand off reverified predecessor text to dependent prompts (up to
16 KiB per artifact and 64 KiB total); it does not mount or copy predecessor files.
Existence/content/hash criteria establish the
specified artifact property; they do not establish repository test correctness.
Registered checks are not available in this release.

Select a task to inspect its criteria, dependencies, attempts and evidence. A
verified badge requires passed evidence for every criterion from the current
attempt and artifact revision. Queued means admitted; Starting means dispatch is
requested or being reconciled. Running means the runtime observed the provider
running. Pause requested lets a running attempt finish while
holding further attempts. Cancel requested remains pending while runtime custody
is reconciled. Controls use the observed revision and require confirmation.

If a mutation response is lost, its outcome is unknown. Refresh and inspect the
server state, then acknowledge the inspection before issuing a new operation.
ShellDeck never silently retries. The private dispatch journal preserves unknown
operations across restarts and is partitioned by connection/account credentials.
Acknowledgment is not proof that the original operation succeeded.

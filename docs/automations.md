# Scheduled tasks and workflows

Open **Automations** from the Dev activity rail, the Go menu, the command palette,
or the terminal welcome screen. Scheduled tasks, Manage workflows and history
share this surface.

## Create a task

1. Choose a starter or **New task**. Review its title and instructions.
2. Choose where it runs:
   - **Manage** uses the selected Manage site's AI provider and keeps running
     while Inklura Desktop is closed. Configure connected sources and advanced
     workflow steps in Manage.
   - **This computer** uses a chosen local, read-only Agent session. Its provider,
     directory, model and permissions remain bound to that session. Use
     **Set up a session in Agents** to configure a session, then return to the
     task form.
3. Choose a recurrence preset or a five-field cron rule and an IANA time zone,
   such as `Europe/Paris`. The one-time option runs at the next matching time.
4. **Save draft**, review the task, then **Activate schedule**. Saving alone
   does not start execution. Editing a local task makes it inactive again.

Starter prompts use available sources. They do not configure external data
feeds or guarantee access to current prices, events or research.

## Run, inspect and pause

Manage's **Test now** uses its workflow test mode. **Open in Manage** opens the
workflow editor; result links open its execution details. Publishing a draft
is separate from saving it. Advanced graphs remain editable in Manage.

For local tasks, **Run now** starts an ordinary read-only agent turn without
activating or advancing the schedule. **Open agent timeline** shows its output
and complete session history. History also shows recent turns of sessions used
by local tasks, including manually started turns.

Local tasks run only while the desktop is open and Dev access is available.
Missed recurring occurrences coalesce into one run rather than replaying every
missed tick. Calendar schedules preserve the selected time zone's wall-clock
time across daylight saving changes. One-time tasks do not replay after restart.

Busy sessions wait. A changed session context, missing or archived session,
failed run, or request for interaction can pause a task. Review the reason and
the agent timeline before editing or activating it again. Automatic scheduled
runs have a ten-minute limit and use the existing session concurrency limits.

Pause a task from its details. Deletion requires confirmation; deleting a local
task keeps its agent session and timeline. Existing interval-based tasks remain
available through the Agents workflow manager. Calendar task files require
a compatible build; older builds refuse them instead of interpreting them as
interval tasks.

## Manage connection

Choose the intended Manage site in the title bar. Manage checks the signed-in
account's current workflow permissions for that site. If **Reconnect account**
appears, sign in again to enable the device's workflow connection. Older device
connections need this once; expired workflow sessions can require it again.

Local task execution remains in the existing Agents runtime. Neither a starter
nor a saved schedule grants additional permissions or starts a provider by itself.

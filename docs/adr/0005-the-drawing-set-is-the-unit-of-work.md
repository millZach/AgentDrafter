# The drawing set is the unit of work, and each sheet takes a change fully or not at all

A job's sheets, block library, templates and data form one drawing set. A change set can span sheets and is undone as one; search, consistency and "what else references this" run across the set. Within a change set each sheet is all-or-nothing: the sheets that can take the change are changed, a sheet that cannot is left exactly as it was, and the set is marked inconsistent until the failed sheets are fixed or the whole change set is undone. In a trial of an existing tool, per-drawing undo left two sheets disagreeing about a tag with no warning; that is what this decision prevents. All-or-nothing across the whole set was rejected because one bad sheet would stop a long unattended run.

## Consequences

- The journal, undo states and change lists live in the set's folder and travel with the job. Old undo states are pruned. A set works with git but does not need it.
- A sheet is never left half-edited.
- A template is a record of every sheet built from it, so a template edit lands only if every one of those sheets takes its whole part; otherwise none of them takes it, and sheets built from other templates still land. Zach decided this on 2026-11-11.
- Until the journal is built, the working revision is the set's own files, edited in place, history is git's, and the inconsistent mark is a file in the set's folder (`docs/change-sets.md`).

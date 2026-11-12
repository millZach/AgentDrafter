# The drawing set is the unit of work, and each sheet takes a change fully or not at all

A job's sheets, block library, templates and data form one drawing set. A change set can span sheets and is undone as one; search, consistency and "what else references this" run across the set. Within a change set each sheet is all-or-nothing: the sheets that can take the change are changed, a sheet that cannot is left exactly as it was, and the set is marked inconsistent until the failed sheets are fixed or the whole change set is undone. In a trial of an existing tool, per-drawing undo left two sheets disagreeing about a tag with no warning; that is what this decision prevents. All-or-nothing across the whole set was rejected because one bad sheet would stop a long unattended run.

## Consequences

- The journal, undo states and change lists live in the set's folder and travel with the job. Old undo states are pruned. A set works with git but does not need it.
- A sheet is never left half-edited.
- A template is a record of every sheet built from it, so a template edit lands only if every one of those sheets takes its whole part; otherwise none of them takes it, and sheets built from other templates still land. Zach decided this on 2026-11-11.
- The working revision is the set's own files, edited in place, and the inconsistent mark is a file in the set's folder (`docs/change-sets.md`). The journal beside it records every change set and failed attempt, and undo puts a change set's files back from it, byte for byte, with every later change set's (`docs/journal.md`).
- A process killed while it puts a change set's files in place leaves no sheet half-edited for long: Zach decided on 2026-11-11 that a change set's, an undo's and an accept's journal entry is written ahead of its files and marked done once all are in place, and the next command that reads the set rolls back an entry left pending, every file byte for byte from the undo state, or refuses, naming the set and the entry, and, as he decided on review, refuses so when a file it would put back was changed by hand since (`docs/journal.md`).

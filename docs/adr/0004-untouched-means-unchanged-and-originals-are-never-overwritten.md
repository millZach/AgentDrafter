# Untouched means unchanged, and an original is never overwritten

The drawings this engine revises are a plant's master records, edited in AutoCAD after we hand them back. So a revision returns everything AgentDrafter did not edit exactly as it was, including content it does not understand (dynamic blocks, hidden data, custom objects), and the original DWG is read-only: every change is saved as a new revision file beside it, with its change list and a report of anything that changed outside what was asked. The easier bars (looks the same, or core content with a warning) were rejected because a plant would find the loss months later.

When the reader cannot fully decode an original, AgentDrafter refuses to save a revision of it and names the records it could not decode. The reviewer can allow the save for that one drawing, and the loss is then listed in the report saved with the revision. "Always refuse" was rejected because it leaves no way to revise a drawing whose loss the reviewer has read and accepted; "save and report" was rejected because a lost record slips through when the report is not read.

## Consequences

- The engine must carry unknown DWG content through a save instead of rebuilding the file from what it understood. This rules out any DWG library that drops what it cannot parse.
- A revision is saved in the DWG version its original arrived in, because unknown content survives only that way (ADR-0006).
- The reader must know, for every drawing, whether it decoded everything, and say so before any change is made.
- Every write is tested by having AutoCAD audit the result and by comparing AutoCAD's own reading of the original and of the revision.
- Job folders hold more files: originals, revisions and their change lists.

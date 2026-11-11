# The journal and undo

The journal is the record, in a drawing set's own folder, of every change set and every failed attempt (see [`CONTEXT.md`](../CONTEXT.md)). A later session, in a new process, reads it to see what was done, and undoes a change set from it as one, across every sheet it touched. It travels with the set's folder and needs no git: the folder need not be a repository.

`agentdrafter journal <drawing-set-folder>` prints the journal, and `agentdrafter undo <drawing-set-folder> <change-set> [--dry-run]` undoes a change set, each as one line of JSON. They are thin layers over the core's `journal` and `undo`, which take and return plain data (ADR-0007).

## Where it is

```
demo-set/
  .agentdrafter/
    journal.jsonl          the journal, one entry per line
    undo-states/           the bytes each file had before a change set wrote it, named by their SHA-256
    inconsistent.yaml      the inconsistent mark, while the set is marked
```

The journal sits beside the [inconsistent mark](change-sets.md#an-inconsistent-set). Once a set has a journal, the `.agentdrafter` folder stays when the mark is taken away.

## What it records

The journal is written in JSON lines: one entry per line, each a JSON object, added at the end in order and never rewritten, so an agent reads it a line at a time. Each entry has a number, counted from 1, and a change set is undone by its entry's number, which the change set's reply gives as `journal_entry`.

```json
{"entry":2,"kind":"change_set","at":"2026-11-11T20:05:29Z","change_set":{"operations":[{"center":[5,5,0],"op":"add_circle","radius":0.5,"sheet":"DEMO-001"},{"center":[5,5,0],"layer":"note","op":"add_circle","radius":0.5,"sheet":"DEMO-002"}]},"landed_on":["DEMO-001"],"failed_on":[{"sheet":"DEMO-002","code":"unknown_name","message":"operation 1 (`add_circle`) on sheet `DEMO-002`: unknown layer `note`; the sheet's layers: 0, trim, border, notes"}],"files":[{"path":"rows.yaml","before":"7b10789f…","after":"1097fc0e…"},{"path":"dwg/DEMO-001.dwg","before":"f1982000…","after":"7cf68f86…"},{"path":".agentdrafter/inconsistent.yaml","before":null,"after":"9296004f…"}]}
```

| Key | What it is |
|---|---|
| `entry` | The entry's number. |
| `kind` | `change_set`, `failed_attempt` or `undo`. |
| `at` | When it was added, in UTC. |
| `change_set` | The change set as it was sent, with its operations and its `fixes`. |
| `landed_on` | The sheets the change set landed on. |
| `failed_on` | The sheets it failed on, each with the error's code and message. |
| `error` | Why a failed attempt refused whole did nothing: the error's code, message and field. |
| `files` | Each file the change set or the undo wrote: its path, relative to the set's folder, and the SHA-256 of its bytes `before` and `after`, `null` where there was no file. |
| `folders_made` | The folders the change set made for its files, such as the output folder of a set never built. |
| `undid` | The change sets an undo undid, the newest first. |
| `pruned` | The change sets whose undo states were pruned when this one landed. |

Keys with nothing to say are left out.

- **A change set** is recorded in the same all-or-nothing step as its files: its records, its DWGs and the mark land with their undo state and the journal's entry, or none of them does. A change set that lands on some sheets and fails on others is a change set: it lists both.
- **A failed attempt** changed nothing. It is a change set refused whole, such as one naming an operation that does not exist, one that failed on every sheet it touched, or one whose files could not be written. It stays in the journal and is never undone.
- **A dry run** changes nothing on disk, the journal included, so it is not recorded.
- **An undo** is recorded with the change sets it undid and the files it put back.

`agentdrafter journal` reads the journal back with what has become of each change set since: `undo_state` is `kept`, `pruned` or `undone`, with `undone_by` or `pruned_by` naming the entry that did it. `can_undo` lists the change sets an undo can take now.

```json
{"ok":true,"result":{"set":"demo-set","file":"demo-set/.agentdrafter/journal.jsonl","message":"the journal records 2 change sets, 0 failed attempts and 0 undos. Change sets 1 and 2 can be undone, each with every later one","entries":[{"entry":1,"kind":"change_set","...":"...","undo_state":"kept"},"..."],"can_undo":[1,2],"kept_undo_states":50}}
```

A line that is not a journal entry is reported as `read_failed`, with its line. A change set does not land on a set whose journal cannot be read, since it could not be undone.

## Undo

**Undo is a stack** (Zach's decision on [#16](https://github.com/millZach/AgentDrafter/issues/16)). Undoing a change set undoes every later change set still in place too, the newest first; there is no undo of one change set out of order. Each file goes back to how it was before the oldest of them that wrote it, byte for byte: records, DWGs, the inconsistent mark, and a file a change set made is taken away, with each folder it made once that folder is empty. The files are put back through the same all-or-nothing step a change set lands through, with the undo's entry in the journal.

```json
{"ok":true,"result":{"set":"demo-set","dry_run":false,"journal_entry":3,"message":"undid change sets 2 and 1, the newest first, since an undo takes every later change set too: 6 files put back as they were before change set 1, byte for byte","undid":[2,1],"files":[{"file":"demo-set/templates/ansi-b.yaml","action":"restored"},"...",{"file":"demo-set/.agentdrafter/inconsistent.yaml","action":"removed"}],"folders_removed":[]}}
```

| Key | What it is |
|---|---|
| `undid` | The change sets undone, the newest first. |
| `files` | Each file put back (`restored`) or taken away because a change set made it (`removed`). |
| `folders_removed` | The folders the change sets made that the undo took away, or for a dry run would take away once empty. |
| `journal_entry` | The undo's own entry; absent for a dry run. |
| `inconsistent` | The set's mark as it stands after the undo, or would after a dry run; absent when the set is consistent. |

**The inconsistent mark is put back as it was** before the change set. Undoing the change set that failed on a sheet clears the failure it marked; undoing a later change set that fixed it brings the mark back.

**A dry run**, `--dry-run`, checks everything an undo checks and reports what it would put back, and writes nothing.

**An issued change set cannot be undone.** Nothing is issued yet: the reviewer's accept comes with the viewer. Until then every change set still in place can be undone.

### A file changed by hand

Zach decided on 2026-11-11 that **undo refuses when a file was changed by hand since the change set.** Before it puts anything back, the undo compares each file it would put back with what the journal says the change sets left behind, by its SHA-256. If any differs, it changes nothing and names each file that differs. A file a change set made that someone has since taken away counts as changed. So does a file changed between two of the change sets it undoes, since putting back the older state would lose that change too.

```json
{"ok":false,"error":{"code":"changed_by_hand","message":"change set 1 cannot be undone: the undo would put back files changed since, by hand or by another program, and lose those changes: `demo-set/rows.yaml` (changed since change set 2 left it); nothing was changed","field":"change_set"}}
```

Put the file back as the change set left it and undo again, or make the change with a change set instead.

## Old undo states are pruned

Zach decided on 2026-11-11 that **old undo states are pruned so that the 50 newest change sets keep theirs.** An undo state is the saved bytes a change set's undo needs: each file it wrote, as it was before. The bytes are kept once each, named by their SHA-256, in `.agentdrafter/undo-states/`, so a file that comes back to bytes kept already takes no more room.

- When a change set lands, every change set it leaves outside the 50 newest has its undo state pruned. The landing change set's entry names them under `pruned`. Failed attempts and undos are not change sets and do not count.
- The journal still records a pruned change set, with its operations, its sheets and its files, and says its undo state was pruned (`undo_state: pruned`, `pruned_by`).
- Undoing a change set whose undo state is pruned, or one that needs a later pruned state, is refused with the error `pruned`, which says which change set's state was pruned and when, and nothing is changed.
- A change set that has been undone needs its undo state no more, so it goes too.

## Errors

| Code | In the journal and undo it means |
|---|---|
| `unknown_name` | The journal has no entry of that number. The message lists the change sets an undo can take. |
| `bad_value` | The entry is a failed attempt or an undo, or the change set was undone already. |
| `pruned` | A change set the undo needs has had its undo state pruned. |
| `changed_by_hand` | A file the undo would put back was changed since the change sets left it. |
| `read_failed` | The set's settings, the journal or an undo state cannot be read, or an undo state is missing or damaged. |
| `write_failed` | A file could not be put back; every file is as it was before the undo. |
| `usage` | The command's arguments are wrong, such as a change set that is not a number. |

## How it works

`crates/agentdrafter-core/src/change/journal.rs` holds the journal: its entries, reading it, a change set's undo state and entry, a failed attempt's entry, and pruning. `undo.rs` holds the undo. Both put their files in place through `commit.rs`, which adds the journal's entry last, once every file is in place: a journal that cannot take it is cut back to its length before, and every file is put back, so nothing lands without its entry. Nothing reads or runs git.

## What is not offered yet

- Issuing: an issued change set cannot be undone, and the reviewer's accept comes with the viewer.
- Redo: an undone change set is applied again by sending it again; its operations are in its entry.
- Two processes changing one set at once: nothing locks the set, so each agent's change sets should go one at a time.
- A record that was a link before a change set comes back as a file holding the same bytes, as the change set left it.
- The MCP server, the Python library and the HTTP API ([#14](https://github.com/millZach/AgentDrafter/issues/14)), which will reach the same core entry points.

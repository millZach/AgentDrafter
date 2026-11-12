# The journal and undo

The journal is the record, in a drawing set's own folder, of every change set and every failed attempt (see [`CONTEXT.md`](../CONTEXT.md)). A later session, in a new process, reads it to see what was done, and undoes a change set from it as one, across every sheet it touched. It travels with the set's folder and needs no git: the folder need not be a repository.

`agentdrafter journal <drawing-set-folder>` prints the journal, `agentdrafter undo <drawing-set-folder> <change-set> [--dry-run]` undoes a change set, and `agentdrafter accept <drawing-set-folder>` issues the working revision ([below](#accept)), each as one line of JSON. They are thin layers over the core's `journal`, `undo` and `accept_with`, which take and return plain data (ADR-0007).

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
| `kind` | `change_set`, `failed_attempt`, `undo` or `accept`. |
| `at` | When it was added, in UTC. |
| `change_set` | The change set as it was sent, with its operations and its `fixes`. |
| `landed_on` | The sheets the change set landed on. |
| `failed_on` | The sheets it failed on, each with the error's code and message. |
| `error` | Why a failed attempt refused whole did nothing: the error's code, message and field. |
| `files` | Each file the change set or the undo wrote: its path, relative to the set's folder, and the SHA-256 of its bytes `before` and `after`, `null` where there was no file. |
| `folders_made` | The folders the change set made for its files, such as the output folder of a set never built. |
| `undid` | The change sets an undo undid, the newest first. |
| `pruned` | The change sets whose undo states were pruned when this one landed. |
| `objects` | Each object the change set added, changed, moved or deleted, on each sheet it landed on, by its ID, with `how`: `added`, `changed` or `deleted`. An edit to a template's object is listed on every sheet built from it, and setting a row's value lists each place on the sheet it fills. The viewer marks them. |
| `via` | The interface an accept came through: `viewer`, `cli`, `http`, `mcp` or `python`. |
| `issued` | The change sets an accept issued. |
| `failed_checks` | The failed checks an accept was told to accept despite: each one's sheet, rule, object's ID and message. |
| `revision` | Each file of the set as an accept issued it: its path, relative to the set's folder, and the SHA-256 of its bytes. |

Keys with nothing to say are left out.

- **A change set** is recorded in the same all-or-nothing step as its files: its records, its DWGs and the mark land with their undo state and the journal's entry, or none of them does. A change set that lands on some sheets and fails on others is a change set: it lists both.
- **A failed attempt** changed nothing. It is a change set refused whole, such as one naming an operation that does not exist, one that failed on every sheet it touched, or one whose files could not be written, which lists every sheet it touched under `failed_on`, each with the error. It stays in the journal and is never undone.
- **A dry run** changes nothing on disk, the journal included, so it is not recorded.
- **An undo** is recorded with the change sets it undid and the files it put back.
- **An accept** is recorded with the interface it came through, the change sets it issued and the files it issued. It writes no file of the set.

`agentdrafter journal` reads the journal back with what has become of each change set since: `undo_state` is `kept`, `pruned` or `undone`, with `undone_by` or `pruned_by` naming the entry that did it, and `issued_by` names the accept that issued it. `can_undo` lists the change sets an undo can take now: those that keep their undo state and are not issued.

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

**An issued change set cannot be undone.** An undo that would take one, the change set asked for or a later one, is refused with the error `issued`, which names the accept that issued it and the interface it came through, and nothing is changed. A change set that lands after an accept can be undone, back to the accept and no further.

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

## Accept

Accept issues the working revision: the reviewer's accept from the viewer, or an agent's on their behalf. Zach decided on 2026-11-11 that **accept is a command like every other**, in the core's registry, so the command line, the HTTP API, the MCP server, the Python library and the viewer all reach it, and an agent can accept when the reviewer asks it to.

```
agentdrafter accept <drawing-set-folder> [--despite-failed-checks] [--newest-seen <entry>] [--dry-run] [--fonts <folder>]...
```

- **It is refused while the set is inconsistent**, with the error `inconsistent`, which says which sheets failed. Fix them, or undo the change set, first.
- **It checks every sheet first**, with the [checker](check.md), from the saved files. When checks fail, it lists them under `checks` and issues nothing: the reply is not ok, with the error `checks_failed`. A caller that has seen them accepts despite them with `despite_failed_checks` (`--despite-failed-checks`); the viewer shows them to the reviewer and asks first. The accept's entry records the failed checks it was accepted despite.
- **It marks every change set still in place issued**: each one not undone that no earlier accept issued, pruned ones among them. An issued change set cannot be undone ([above](#undo)).
- **The journal records it, with the interface it came through**, `viewer`, `cli`, `http`, `mcp` or `python`, so the reviewer can tell an agent's accept from their own. The interface is the one the command reached the core through; the input does not say it. The viewer's page reaches the core through the HTTP API, and a request it sends is the viewer's ([servers.md](servers.md#the-http-api)).
- **It records each file of the set as it issues it**, under `revision`: the settings, the rows file, each template the rows name and the block library each names, and each sheet's DWG in the output folder. A PNG or a PDF made from those files, byte for byte, says in its stamp that it came from an issued revision, and which accept issued it ([the stamp](render.md#the-stamp)).
- **`newest_seen`** (`--newest-seen`) is the journal's newest entry the caller has looked at: the accept is refused with the error `moved_on` if the journal holds a newer one, so a change set that landed while the reviewer was deciding is not issued unseen. The viewer always sends it.
- **There is nothing to accept** when no change set has landed since the last accept and every file of the set is as it issued it: the error `nothing_to_accept`. A set no change set has touched can still be accepted once: that issues the set as it was built.
- **A dry run**, `--dry-run`, checks and reports everything an accept would do, and records nothing.

```json
{"ok":true,"result":{"set":"demo-set","dry_run":false,"journal_entry":5,"via":"cli","message":"accepted: issued change sets 1 and 4, which landed on `DEMO-001`, `DEMO-002` and `DEMO-003`, and the set's 7 files, recorded in entry 5; an issued change set cannot be undone. Every check passes.","issued":[1,4],"sheets":["DEMO-001","DEMO-002","DEMO-003"],"checks":{"...":"..."},"revision":[{"path":"set.yaml","sha256":"4e78…"},"..."]}}
```

The accept's entry in the journal:

```json
{"entry":5,"kind":"accept","at":"2026-11-11T21:40:02Z","via":"viewer","issued":[1,4],"revision":[{"path":"set.yaml","sha256":"4e78…"},{"path":"rows.yaml","sha256":"7b10…"},"..."]}
```

## The set as it was at an entry

The viewer steps back through the journal and shows the set as it was just after each entry, read-only. `agentdrafter display-list <set> --entry <n>` (`entry` in the request) draws a sheet so: the set's files are rebuilt in memory from the undo states, and nothing is written. Each file a later entry wrote is as the next entry to write it found it, its bytes `before`, which the undo states hold, or the file on disk when it is still those bytes; a file no later entry wrote is as it is now. A file is named as the journal names it, by the path the set's reader and a change set both join onto the set's folder and the journal takes back off it, so a template or a rows file outside the set's folder, such as `../shared/rows.yaml`, or one written with `..`, is drawn from the bytes the journal kept, and never from the file on disk as it is now.

An entry cannot be shown once the bytes it needs are gone: when the undo state of a later change set that held a file as it was has been pruned, or that change set was undone, since the journal keeps no copy of a file as an undone change set left it. The reply is then the error `pruned`, which names the change set and why. Since only the 50 newest change sets keep their undo states, the entries before the newest pruned change set cannot be shown; the newest pruned one still can, from the undo states of the change sets after it.

## Errors

| Code | In the journal and undo it means |
|---|---|
| `unknown_name` | The journal has no entry of that number. The message lists the change sets an undo can take. |
| `bad_value` | The entry is a failed attempt, an undo or an accept, or the change set was undone already. |
| `issued` | A change set the undo would take was issued by an accept. |
| `pruned` | A change set the undo needs has had its undo state pruned; or, drawing the set as it was at an entry, a file it needs is no longer kept. |
| `changed_by_hand` | A file the undo would put back was changed since the change sets left it. |
| `read_failed` | The set's settings, the journal or an undo state cannot be read, or an undo state is missing or damaged. |
| `write_failed` | A file could not be put back; every file is as it was before the undo. |
| `usage` | The command's arguments are wrong, such as a change set that is not a number. |
| `inconsistent` | An accept: the set is inconsistent. |
| `checks_failed` | An accept: checks failed, and it was not told to accept despite them. The reply lists them, and nothing was issued. |
| `moved_on` | An accept: the journal holds an entry newer than `newest_seen`. |
| `nothing_to_accept` | An accept: the working revision is the one the last accept issued. |

## How it works

`crates/agentdrafter-core/src/change/journal.rs` holds the journal: its entries, reading it, a change set's undo state and entry, a failed attempt's entry, an accept's entry, and pruning. `undo.rs` holds the undo, `accept.rs` the accept and what a stamp says of the revision its files came from, and `history.rs` the set's files as they were at an entry. Both put their files in place through `commit.rs`, which adds the journal's entry last, once every file is in place: a journal that cannot take it is cut back to its length before, and every file is put back, so nothing lands without its entry. If the journal cannot be cut back either, the error does not say nothing was changed: it names the journal and the entry it may still hold for the change that was rolled back. Nothing reads or runs git.

## What is not offered yet

- Redo: an undone change set is applied again by sending it again; its operations are in its entry.
- Two processes changing one set at once: nothing locks the set, so each agent's change sets should go one at a time.
- A record that was a link before a change set comes back as a file holding the same bytes, as the change set left it.
- Revisions named by letter, such as Rev B, as the picked prototype named them: an accept is known by its entry in the journal.

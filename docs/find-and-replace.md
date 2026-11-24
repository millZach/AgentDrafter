# Find and replace across a drawing set

AgentDrafter supports batch text find-and-replace and query-driven batch attribute editing across a drawing set (and on original `.dwg` revisions) through two change-set operations:

- **`replace_text`** (AutoCAD alias **`FIND`**) — replaces text across single-line `text` objects, multiline `mtext` objects, and editable insert `attributes` across named sheets or the entire drawing set.
- **`set_attributes`** (AutoCAD alias **`GATTE`**) — sets an attribute's value on every insert selected by a `where` query across named sheets or the entire drawing set.

Both operations go through the core's change-set registry (ADR-0007), validate every touched sheet and rebuild its DWG atomically (ADR-0005), support `--dry-run` previews that list every place that would change without writing, and record full journal entries with byte-for-byte `undo`.

---

## `replace_text` (`FIND`)

```json
{
  "operations": [
    {
      "op": "replace_text",
      "find": "JB-101",
      "replace": "JB-201",
      "match": "whole_word"
    }
  ]
}
```

### Keys

| Key | Required | Default | Meaning |
|---|---|---|---|
| `op` | yes | — | `"replace_text"` or `"FIND"` (any case). |
| `find` | yes | — | Non-empty string or regular expression pattern to search for. |
| `replace` | yes | — | Replacement string (may be empty `""`). In `pattern` mode, `$1`, `${name}`, and `$$` expand capture groups; in `exact` and `whole_word` modes it is inserted literally. |
| `match` | no | `"exact"` | `"exact"` (literal substring), `"whole_word"` (literal match bounded on both sides by non-word characters, i.e. neither alphanumeric nor `_`), or `"pattern"` (Rust regular expression, compiled with a 1 MiB size limit). |
| `case_sensitive` | no | `true` | Set `false` for case-insensitive matching. |
| `sheets` | no | all sheets | Non-empty list of sheet names to search and edit. |
| `in` | no | `["text", "mtext", "attribute"]` | Non-empty subset of `"text"`, `"mtext"`, and `"attribute"` restricting which places are searched. |
| `tag` | no | any tag | When replacing in attributes, restrict matches to this attribute tag (case-insensitive). |
| `block` | no | any block | When replacing in attributes, restrict matches to inserts of this block (case-insensitive). |

### Behaviour on templates, rows, and locked layers

- **Row-bound values (`{row: NAME}` and lists of parts):** Replacing text in a template object backed by `{row: NAME}` updates that sheet's row value once (even when multiple objects on the sheet, such as a title block attribute and a heading text, share that row value), never double-replacing, and reports every affected object in `replacements`.
- **Literal template text:** When a match sits directly inside a shared template (not from a row), `replace_text` edits the template once if all sheets built from that template are in scope (e.g. `sheets` is omitted or includes them all). If `sheets` names only a strict subset of a shared template's sheets, the operation refuses with `bad_value` naming the template and the other sheets so nothing is silently altered outside `sheets`.
- **Constant attributes:** Constant block attributes cannot vary per insert and are skipped.
- **Locked layers:** Any matched object (or any place filled by an edited row value) on a locked layer fails with `locked`.
- **No matches:** Finding zero matches is not an error; the operation succeeds with `replacements: []`, touches no sheets, and writes no files.

### Reply

Each matched place is listed under the operation's `replacements`:

```json
{
  "sheet": "LOOP-001",
  "id": "LOOP-001/tb",
  "kind": "attribute",
  "tag": "TITLE",
  "before": "AREA 100 JB-101 FLOW FT-101",
  "after": "AREA 100 JB-201 FLOW FT-101",
  "count": 1,
  "row_value": "TITLE"
}
```

With `--dry-run`, `agentdrafter change <set> <change-set.json> --dry-run` returns the exact `replacements` list and touched sheets without modifying any file.

---

## `set_attributes` (`GATTE`)

```json
{
  "operations": [
    {
      "op": "GATTE",
      "where": { "block": "TITLE_BLOCK" },
      "tag": "DATE",
      "value": "2026-11-24"
    }
  ]
}
```

### Keys

| Key | Required | Default | Meaning |
|---|---|---|---|
| `op` | yes | — | `"set_attributes"` or `"GATTE"` (any case). |
| `where` | yes | — | Query selection object choosing which inserts to edit — accepts the exact same keys as `query attributes` (`id`, `name`, `block`, `tag`, `value`, `text`, `match`, `window`, `crossing`, `touching`, `space`). `{}` selects every insert on the targeted sheets. |
| `tag` | yes | — | Attribute tag to set (case-insensitive, stored uppercase). |
| `value` | yes | — | New single-line attribute value. |
| `occurrence` | no | — | 1-based occurrence index when a block defines `tag` more than once. |
| `sheets` | no | all sheets | Non-empty list of sheet names to search and edit. |
| `row_value` | no | — | Row value name to set when the template joins the attribute from a list of parts. |

### Reply

Each updated insert is listed under the operation's `attributes_set`:

```json
{
  "sheet": "LOOP-001",
  "id": "LOOP-001/tb",
  "block": "TITLE_BLOCK",
  "tag": "DATE",
  "before": "2026-10-01",
  "after": "2026-11-24",
  "row_value": "DATE"
}
```

---

## Original `.dwg` revisions

Both `replace_text` (`FIND`) and `set_attributes` (`GATTE`) also work when revising an original `.dwg` file with `agentdrafter change <original.dwg> <change-set.json>`. Every untouched object keeps its raw DWG bytes and handle verbatim (ADR-0004), and only matched texts, mtexts, and attributes are updated in place.

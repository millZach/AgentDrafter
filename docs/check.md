# The checker

The checker re-reads a drawing set's saved files, or one sheet description, and reports each [check](../CONTEXT.md) a sheet fails (see [`CONTEXT.md`](../CONTEXT.md)). It is separate from whatever made a change: it reads the records on disk, as a build reads them, never the records another part held in memory, so whatever made a change never marks its own work (`docs/plan.md`). Its default checks are the ones a reviewer would otherwise catch by eye: the exact paper size, nothing off the sheet, no text overflowing its cell, and no text sitting on other text.

`agentdrafter check <drawing-set-folder | sheet-description> [--sheet <sheet>]... [--fonts <folder>]...` checks every sheet of a set, or only those `--sheet` names, in any case, and prints one line of JSON. It is a thin layer over the core's `check_with`, which takes a request and returns the report as plain data (ADR-0007). A change set's reply carries the checker's reply too ([below](#after-a-change-set)).

A failed check is reported, not refused. The reply is `ok`, with each failed check under `findings`; it does not stop a change set from landing, and the reviewer can accept a set after a warning that lists them (`docs/plan.md`). A sheet that cannot be read stops the check with its error, placed in its file, as a query does: a report that left a sheet out could say it passed.

## What the checker reads

The checker reads each sheet's record: the sheet description, or the template filled in from its row with the row's own objects, and the block library it names, from disk. It does not read the built DWGs, so it needs no build and cannot read a stale one, and a file changed on disk by hand is checked as it now is. It writes nothing.

Each sheet is drawn once through the one drawing pipeline (ADR-0011), as [`query extents`](query.md#extents) draws it: every layer continuous, curves drawn as chords within a millionth of the sheet's larger side, and text as its font draws it. Every check measures that drawing, so what it measures is what the PNG, the PDF and the viewer draw. Text is measured in the fonts the font folders hold, or a look-alike, and the reply lists each look-alike under `font_substitutions`, as a build does.

## The paper and the cells

Two keys, decided by Zach on 2026-11-11, say what a sheet is held to. Only the checker reads them: a build, a render and a plot are the same with them or without them, and a DWG is unchanged by them.

**A sheet's paper is declared** under `paper` in its template, or in a sheet description built on its own ([The sheet description](sheet-description.md#paper)): its width and height, `paper: [17, 11]`, in the declared unit or each with its own, or a standard size and the way it lies, `paper: ANSI B landscape`. The standard sizes are ANSI A to E (8.5 by 11, 11 by 17, 17 by 22, 22 by 34 and 34 by 44 inches) and ISO A0 to A4 (841 by 1189, 594 by 841, 420 by 594, 297 by 420 and 210 by 297 mm); `landscape` lays the longer side across and `portrait` up, and a size without one is an error that names both. The paper's lower left corner is at the origin, `[0, 0]`, where a sheet drawn in model space at its paper's size draws its trim line from, as the demo set's template does. The set's settings take no paper: the template draws the border, so it is the one place that knows what the border is drawn for.

**A title-block cell is declared in the block library**, under `cell` on an attribute definition or on a text of a block ([The block library](block-library.md#cells)): two opposite corners of a box in the block's own coordinates, `cell: [[-6.5, 0.6, 0], [0, 1.4, 0]]`. On each insert the cell is moved, scaled and turned with the insert. A text or an attribute with no cell is not checked for overflow.

## The checks

Each check is held to one tolerance, **0.01 mm on paper**, given in the reply's declared unit as `tolerance`: a sheet is drawn at its paper's size, so a length on it is a length on paper. Within it, a sheet is its paper's size, an object is on the paper, a text is in its cell, and two texts do not sit on each other.

| Rule | A sheet fails it when |
|---|---|
| `paper_size` | Its extents, the box round everything it draws as `query extents` gives it, are not its paper's width and height, each within the tolerance; or it draws nothing. **A sheet that declares no paper fails it once**, with a finding that says so and what the sheet draws, and is not checked for objects off the paper: it is not silently skipped. |
| `off_sheet` | One of its objects draws past the paper, from `[0, 0]` to the paper's width and height, by more than the tolerance. An insert is one object: what its block draws and its attributes together. |
| `text_overflow` | A text or an attribute an insert draws, whose block declares its cell, draws past the cell. What it draws, as its font draws it, is taken back into the block's own coordinates by the insert's position, scale and rotation, and held to the cell there; how far it reaches past a side, enlarged by the insert's scale, is held to the tolerance. An attribute that draws nothing, being empty or invisible, is not checked. |
| `text_on_text` | Two texts on it have drawn boxes that overlap by more than the tolerance. Each text's box is the box round what it draws, seen in the text's own direction, so text turned with its insert or on its own is boxed as it runs; how far two boxes overlap is how far one would move to clear the other, the least overlap along the sides of either. Every text counts: the sheet's text and mtext, each mtext as one box round all its lines, and each text, mtext and visible attribute an insert draws, against the others of the same insert too. The text drawn later is named as sitting on the one drawn before it. |

## The reply

```json
{"ok":true,"result":{"source":"examples/demo-set","units":"in","sheets":["DEMO-001","DEMO-002","DEMO-003"],"message":"every check passes on the 3 sheets checked","passed":true,"tolerance":0.00039370078740157485,"count":0,"findings":[],"font_substitutions":[{"style":"notes","font":"romans.shx","used":"Hershey Roman Simplex","why":"no font folder is set"},"..."]}}
```

| Key | What it is |
|---|---|
| `source` | The drawing set's folder, or the sheet description, checked. |
| `units` | The declared unit every length in the report is in. |
| `sheets` | The sheets checked, in the set's order. |
| `message` | What was found, in words. |
| `passed` | Whether every check passed on every sheet checked. |
| `tolerance` | How far past a limit counts, 0.01 mm on paper, in the declared unit. |
| `count` | How many checks failed. |
| `findings` | Each failed check, sheet by sheet in the set's order, and on each sheet `paper_size`, `off_sheet`, `text_overflow`, then `text_on_text`. |
| `font_substitutions` | Each text style whose font was not in a font folder, and the look-alike its text was measured with. |
| `inconsistent` | The set's [inconsistent mark](change-sets.md#an-inconsistent-set), while it is marked. |

**A finding** names the sheet, the rule and the object, and says what is wrong in numbers in the declared unit:

```json
{"sheet":"DEMO-001","rule":"text_overflow","message":"the attribute TITLE of the insert `DEMO-001/objects[2]` (named `title-block`) saying `AREA 100 GENERAL ARRANGEMENT OF THE NORTH BUILDING` draws 0.338571 in past the left side and 0.338571 in past the right side of its cell: in the block TITLE_BLOCK's own coordinates it draws from [-6.838571, 0.9] to [0.338571, 1.1], and its cell runs from [-6.5, 0.6] to [0, 1.4] in","object":{"id":"DEMO-001/objects[2]","name":"title-block","type":"insert","file":"demo-set/templates/ansi-b.yaml","field":"objects[2]","line":19,"column":5,"in_block":{"block":"TITLE_BLOCK","file":"demo-set/templates/../blocks.yaml","field":"blocks.TITLE_BLOCK.objects[13]","type":"attdef","tag":"TITLE"},"text":"AREA 100 GENERAL ARRANGEMENT OF THE NORTH BUILDING"},"drawn":{"min":[-6.838571428571431,0.8999999999999999],"max":[0.33857142857143074,1.0999999999999999],"width":7.1771428571428615,"height":0.19999999999999996},"cell":{"min":[-6.5,0.6],"max":[0.0,1.4],"width":6.5,"height":0.7999999999999999},"past":{"left":0.33857142857143074,"bottom":0.0,"right":0.33857142857143074,"top":0.0}}
```

| Key | What it is |
|---|---|
| `sheet` | The sheet. |
| `rule` | The check it failed: `paper_size`, `off_sheet`, `text_overflow` or `text_on_text`. |
| `message` | What is wrong, in words and numbers. |
| `object` | The object that fails it, by its ID, its name when it has one, its type and where it is written (`file`, `field`, `line`, `column`), as a [query](query.md#find) gives it. For text an insert draws, `in_block` says which object of its block: the block, the block library's `file`, its place in the block (`field`), its type and an attribute's `tag`. `text` is what the text says. None for `paper_size`, a check of the whole sheet. |
| `other` | For `text_on_text`, the text the object sits on. |
| `drawn` | The box round what is drawn: the sheet's extents for `paper_size`, the object's for `off_sheet` and `text_on_text`, the text's in its block's own coordinates for `text_overflow`. `other_drawn` is the other text's. |
| `paper` | The paper declared, for `paper_size` and `off_sheet`: its `width`, `height` and `name` if it was declared by one. |
| `cell` | For `text_overflow`, the cell, in its block's own coordinates. |
| `off_by` | For `paper_size`, `[width, height]`: how much wider and taller the sheet draws than its paper, less than 0 where it is narrower or shorter. |
| `past` | For `off_sheet` and `text_overflow`, how far what is drawn reaches past each side, `left`, `bottom`, `right` and `top`, of the paper or of the cell in its block's own coordinates; 0 on a side it keeps within. |
| `overlap` | For `text_on_text`, how far the two drawn boxes overlap. |

## After a change set

Once a change set has landed, its reply carries the checker's reply under `checks`, on the sheets it landed on ([Change sets](change-sets.md#the-reply)). The change set writes its records and DWGs first; then the checker reads those sheets back from the saved files, exactly as `agentdrafter check <set> --sheet <sheet>...` would, and never the records the change set held in memory. A sheet the change set failed on was left as it was, so it is not checked again.

```json
{"ok":true,"result":{"set":"demo-set","landed":1,"failed":0,"...":"...","checks":{"ok":true,"result":{"source":"demo-set","units":"in","sheets":["DEMO-003"],"message":"2 checks failed on sheet `DEMO-003`: paper_size, off_sheet","passed":false,"...":"...","findings":[{"sheet":"DEMO-003","rule":"paper_size","...":"..."},{"sheet":"DEMO-003","rule":"off_sheet","object":{"id":"DEMO-003/edge","...":"..."},"...":"..."}]}}}}
```

A failed check does not make the change set's reply not `ok`, and does not stop the change set from landing. `checks` is absent for a dry run, which saves nothing for the checker to read, and when the change set landed on no sheet. Should the checker not be able to read a sheet it landed on, `checks` holds its error: `{"ok":false,"error":{...}}`.

## Errors

Errors take the shape and codes a sheet description's do ([Errors](sheet-description.md#errors)).

| Code | In a check it means |
|---|---|
| `unknown_name` | A sheet the set does not hold; the message lists its sheets. |
| `usage` | The command was called wrongly. |
| any other | A sheet that cannot be read, with its own error in its file. |

## How it works

`crates/agentdrafter-core/src/check/mod.rs` reads the set's settings and rows and each sheet's record, as a query does, and writes the report; `check/rules.rs` draws each sheet with `query::measuring_drawing`, the drawing `query extents` measures, and holds it to the four rules. The drawing pipeline marks each shape an insert draws with the object of its block that drew it, so a text and an attribute of a title block are measured apart. `description/paper.rs` reads `paper`, and `description/blocks.rs` reads `cell`.

## What is not offered yet

- Checks a set chooses, rule sets of its own, and checks of a trade, which belong in a generator (ADR-0002).
- A dry run's checks: the checker reads saved files, and a dry run saves none.
- A paper placed anywhere but at the origin, or a paper in the set's settings.
- A cell for a text or mtext on a sheet, outside a block, or for a block's mtext; and checking an mtext line by line rather than as one box.
- Text sitting on lines or other objects: `text_on_text` holds text to text.
- Checking DWGs, built or someone else's.

# New sheets are recorded as text; an existing drawing's record stays its DWG

A new sheet is authored as a sheet description, a text file that states what is on the sheet, and is built into a DWG. A drawing someone else made keeps its DWG as the record: AgentDrafter applies a recorded change list to it and keeps a text mirror beside it for reading and comparing. We did not make text the record for everything, because converting a plant's drawing into our text and back would make "untouched means unchanged" depend on carrying every unknown object through a format of ours. We did not make the DWG the record for everything, because generated sheets are best kept as text that can be read, compared, templated and merged.

## Consequences

- Sheet descriptions are written in a strict subset of YAML. A set shares one block library and its templates, so a sheet can be a template plus one row of data.
- A text mirror is derived and can be thrown away. Editing it changes nothing.
- The engine has two ways of changing a drawing: building from a description, and applying operations to an existing DWG. Both end in the same checks and the same outputs.

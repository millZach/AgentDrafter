# New sheets are written in the 2018 DWG format, and a revision keeps the version it arrived in

AgentDrafter reads every DWG version. A new sheet is written in the 2018 format, which every AutoCAD from 2018 through 2027 saves by default. A revision of an existing drawing is saved in the version its original arrived in. A job can ask for a set to be converted to 2018, or saved down for a plant on an older AutoCAD.

We first chose "always write 2018": a census of a real archive of about 22,000 drawings found nine versions in circulation with a tenth of the files in the current one, and AutoCAD itself saves an old drawing in its current format unless told otherwise. The bake-off (ADR-0009) changed that for revisions. Content the engine does not understand can only be carried through unchanged in the version it was read in. Saved in the version they arrived in, 98.4% of drawn objects came back identical and every drawing in the sample could be written. Converted to 2018, 95.4% came back identical, every drawn object in the oldest drawings changed, and 8 of 53 drawings holding add-on objects could not be converted at all.

## Consequences

- Every version a plant sends must be written well, not only read. Each one needs proof against AutoCAD.
- A drawing set built from existing drawings can hold more than one DWG version.
- Converting an existing drawing to another version is a separate step that someone asks for, and its losses are reported. It is never a side effect of a revision.

# The repository is public and never holds client material

AgentDrafter is open source and public from its first commit. No plant's drawing, block, tag or name enters the repository, including in tests, fixtures, screenshots, logs and notes. Tests and examples use the demo plant, which is made up. Checks against real drawings run only on the maintainer's own machines and report counts and neutral IDs. AutoCAD is used as the judge of what AgentDrafter writes, on a machine that has a licence; it is never part of a job and never part of CI.

## Consequences

- The first usable version is proven twice: on the demo plant in CI on Windows, Linux and macOS, and privately by regenerating a real past job whose drawings pass AutoCAD's audit.
- A bug found on a real drawing is reproduced on a made-up drawing before a test for it is committed.
- AutoCAD's SHX fonts are Autodesk's files and are not in the repository. AgentDrafter reads them where AutoCAD is installed and uses open look-alikes elsewhere.

# The Python library and the pip package

The Python library is the fourth of AgentDrafter's interfaces (see [`CONTEXT.md`](../CONTEXT.md)), beside the command line, the HTTP API and the MCP server ([`servers.md`](servers.md)). It is a binding to the core, a native extension built with [PyO3](https://pyo3.rs), not a wrapper round the command or the HTTP API, and it is as thin as the other three (ADR-0007): it hands a command's input to the core and gives back the core's reply.

One pip package per system carries all of it: the Rust core, the `agentdrafter` command with both servers, and the library. Nothing is published yet (`docs/plan.md`, Install); until the first release the package is built from a checkout.

## The library

```python
import agentdrafter

built = agentdrafter.build(source="examples/demo-plant")
checked = agentdrafter.check({"source": "examples/demo-plant"})
if not checked["result"]["passed"]:
    for finding in checked["result"]["findings"]:
        print(finding["sheet"], finding["rule"], finding["message"])
```

**Every command of the registry is a function**, named as the command is with `-` written `_`: `build`, `schema`, `measure_text`, `render`, `display_list`, `plot`, `stale`, `query`, `operations`, `change`, `journal`, `undo`, `accept` and `check`. The functions are made from the registry when the library is imported, so a command added to the core is in the library with no code of its own, and the library has no other public name. Each function's docstring is what its command does and the keys of its input, from the registry; `help(agentdrafter.plot)` shows it.

**The input** is the command's input as the HTTP API and the MCP server take it (`servers.md`), given as keywords or as one dict: `agentdrafter.undo(set="examples/demo-set", change_set=3)` and `agentdrafter.undo({"set": "examples/demo-set", "change_set": 3})` are the same call. A path may be a `pathlib.Path`. A number reaches the core with every digit it has, as through the command line: every interface reads JSON with each number correctly rounded. A key the command's schema does not name is refused by the core, with a `usage` error, as through every interface. A value no JSON can hold, such as an object or a NaN, is refused with `TypeError` or `ValueError` before it reaches the core.

**The reply** is the one the command line prints, as a dict: `{"ok": True, "result": ...}`, or `{"ok": False, "error": {"code", "message", ...}}` with the `result` of what was done, if anything was. A command's error is its reply, not an exception, so a sheet that failed reads as it does through every other interface. The same input gives the same reply through all four interfaces, and the same change set the same journal entry, apart from its time. An accept records the interface it came through, `python` for the library.

**Paths** are taken as the command line takes them: relative to the current folder, or absolute. **Fonts and plot styles** are read from the folders an input's `fonts` and `plot_styles` name, then from those `AGENTDRAFTER_FONTS` and `AGENTDRAFTER_PLOT_STYLES` list when the function is called.

A command runs without holding Python's lock, so a long plot does not stop a program's other threads. Two change sets on one drawing set should still go one at a time, as through the command line.

## The command

`pip install` puts the `agentdrafter` command beside the library. It is the command line's own code, `crates/agentdrafter-cli`, run inside the library's extension: `agentdrafter build`, `agentdrafter serve` and `agentdrafter mcp` take the same arguments, print the same JSON and exit with the same codes as the binary `cargo run -p agentdrafter-cli` builds. `python -m agentdrafter` runs it too. Ctrl-C stops it at once, as it stops the binary.

The command is a Python entry point that calls into the extension, rather than a second binary in the package, so the package carries one copy of the core, not two, and the command cannot drift from the library: both are the same extension, built from the same commit. What it costs is Python's start-up, about 25 ms a call on forge against 1 ms for the binary; a program that runs many commands uses the library or a server instead.

## Which Python, which systems

The extension is built against Python's stable ABI as Python 3.11 has it (PyO3's `abi3-py311`), so one wheel per system serves Python 3.11 and every later version. 3.11 is the oldest Python that is still supported upstream. CI builds a wheel on Windows, Linux and macOS, as GitHub's runners are: x86-64 Windows and Linux, and Apple silicon macOS.

## Build it from a checkout

The build backend is [maturin](https://www.maturin.rs), named in `pyproject.toml` at the top of the repository. In a virtual environment inside the checkout:

```
python -m venv .venv/build
.venv/build/bin/python -m pip install "maturin==1.16.0"
.venv/build/bin/python -m maturin build --release --out dist
python -m venv .venv/try
.venv/try/bin/python -m pip install dist/agentdrafter-*.whl
.venv/try/bin/agentdrafter check examples/demo-plant
```

(`Scripts` in place of `bin` on Windows.) `pip install .` from the checkout builds and installs it in one step.

The code is in three places: `crates/agentdrafter-py`, the extension, which lists the registry and runs a command; `python/agentdrafter`, the library's Python, which makes a function of each command; and `crates/agentdrafter-cli/src/lib.rs`, the command line, which both the binary and the package's command run.

## Tests

`python/tests` runs against the installed package, from the checkout, with the Python of the virtual environment it is installed in:

```
.venv/try/bin/python -m unittest discover -s python/tests
```

`test_interfaces.py` runs every command of the registry through the library, the installed command, its HTTP API and its MCP server, and holds the four to the same reply and the same files; then the same change sets through all four, held to the same replies and journal entries, apart from their times; and a height with every digit an f64 holds, held to the same reply through the library as through the command, byte for byte. Its calls are the Rust interface test's own, `crates/agentdrafter-cli/tests/calls.json`, so a command added to the registry fails both tests until it has a call there. `test_library.py` tests what the library adds: the form of a call.

CI's `wheel` jobs build the wheel on each system, install it into clean virtual environments of Python 3.11 and 3.14, build and check a copy of the demo plant with the installed command and another through the library, compare their DWGs byte for byte, and run these tests on both Pythons. The wheel is kept as the run's artifact for seven days. Nothing is uploaded to PyPI or any other index.

## Not yet

- Publishing: to PyPI, and plain binaries on GitHub Releases, at the first release, when Zach says so. A wheel for PyPI's Linux needs a `manylinux` build, which works on older systems than the one it was built on; CI's Linux wheel is tagged for the runner's own system.
- Wheels for Intel macOS, Arm Windows and Arm Linux.
- Type stubs, so an editor knows each function's keys.

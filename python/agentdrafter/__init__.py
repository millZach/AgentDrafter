"""AgentDrafter's Python library: every command of the core, as a function.

Each command of the core's registry is a function of this module, named as
the command is, with `-` written `_` (`measure-text` is `measure_text`). It
takes the command's input, the JSON object the HTTP API takes as a body and
the MCP server as a tool's arguments, as keywords or as one dict, and gives
back the reply the command line prints, as a dict: ``{"ok": True,
"result": ...}``, or ``{"ok": False, "error": {"code": ..., "message":
...}}`` with the ``result`` of what was done, if anything was. A command's
error is its reply, not an exception::

    import agentdrafter

    reply = agentdrafter.build(source="examples/demo-plant")
    reply = agentdrafter.check({"source": "examples/demo-plant"})
    if not reply["result"]["passed"]:
        ...

The library is a binding to the core, as thin as the other interfaces
(docs/python.md): its functions are made from the registry when it is
imported, so a command added to the core is here with no code of its own.
Paths are taken as the command line takes them, relative to the current
folder or absolute, and may be given as ``pathlib.Path``. Fonts and plot
styles are read from the folders an input's ``fonts`` and ``plot_styles``
name, then from those ``AGENTDRAFTER_FONTS`` and
``AGENTDRAFTER_PLOT_STYLES`` list when the function is called. A value no
JSON can hold, such as an object or a NaN, is refused with ``TypeError`` or
``ValueError`` before it reaches the core.
"""

import json as _json
import os as _os
import textwrap as _textwrap
from collections.abc import Mapping as _Mapping

from . import _native

__all__ = []


def _plain(value):
    """`value` as plain data, for a value JSON has no form of its own for."""
    if isinstance(value, _os.PathLike):
        path = _os.fspath(value)
        if isinstance(path, str):
            return path
    if isinstance(value, _Mapping):
        return dict(value)
    raise TypeError(
        f"{type(value).__name__} is not plain data: an input holds text, numbers, "
        "true or false, None, lists and dicts with text keys, and paths"
    )


def _doc(info):
    """A function's docstring: what its command does and its input's keys."""
    width = 76
    lines = [_textwrap.fill(info["does"], width), ""]
    lines.append("Its input, as keywords or as one dict:")
    schema = info["input_schema"]
    required = set(schema.get("required", []))
    for key, about in schema.get("properties", {}).items():
        head = f"{key} (required)" if key in required else key
        description = about.get("description", "") if isinstance(about, dict) else ""
        lines.append("")
        lines.append(
            _textwrap.fill(
                f"{head}: {description}" if description else head,
                width,
                initial_indent="    ",
                subsequent_indent="        ",
            )
        )
    if not schema.get("properties"):
        lines.append("")
        lines.append("    nothing.")
    return "\n".join(lines) + "\n"


def _command(info):
    """The function that runs the command `info` lists."""
    name = info["name"]

    def command(input=None, /, **keys):
        if input is None:
            input = keys
        elif keys:
            raise TypeError(
                f"{command.__name__}() takes its input as keywords or as one dict, not both"
            )
        text = _json.dumps(input, default=_plain, allow_nan=False)
        return _json.loads(_native.run(name, text))

    command.__name__ = command.__qualname__ = name.replace("-", "_")
    command.__module__ = __name__
    command.__doc__ = _doc(info)
    return command


for _info in _json.loads(_native.commands()):
    _function = _command(_info)
    globals()[_function.__name__] = _function
    __all__.append(_function.__name__)
del _info, _function

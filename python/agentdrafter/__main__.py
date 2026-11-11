"""The `agentdrafter` command, as the pip package installs it.

It is the command line's own code, run inside the library's extension, so
the command, its HTTP API (`agentdrafter serve`) and its MCP server
(`agentdrafter mcp`) are the binary's: the same arguments, the same JSON on
standard output and the same exit codes. `python -m agentdrafter` runs it
too.
"""

import signal
import sys

from agentdrafter import _native


def main():
    # Ctrl-C stops the command at once, as it stops the binary. Python's own
    # handler would wait for the core to come back, which a server never does.
    signal.signal(signal.SIGINT, signal.SIG_DFL)
    sys.exit(_native.main(sys.argv[1:]))


if __name__ == "__main__":
    main()

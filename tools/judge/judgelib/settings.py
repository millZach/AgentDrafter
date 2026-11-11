"""Where the judge's private things are. Nothing private has a default: each setting comes from an
environment variable or from a config file outside the repository, and the environment wins."""

from __future__ import annotations

import os
import re
from dataclasses import dataclass
from pathlib import Path

DEFAULT_ACCORECONSOLE = r"C:\Program Files\Autodesk\AutoCAD 2026\accoreconsole.exe"

# setting -> environment variable
ENV = {
    "dir": "AGENTDRAFTER_JUDGE_DIR",
    "host": "AGENTDRAFTER_JUDGE_HOST",
    "scratch": "AGENTDRAFTER_JUDGE_SCRATCH",
    "accoreconsole": "AGENTDRAFTER_JUDGE_ACCORECONSOLE",
}
CONFIG_ENV = "AGENTDRAFTER_JUDGE_CONFIG"

HELP = {
    "dir": "the judge's private folder on this machine (the sample in orig/, AutoCAD's readings, results)",
    "host": "the ssh name of the Windows machine that has AutoCAD",
    "scratch": "a scratch folder on that machine, such as C:\\Users\\<you>\\AppData\\Local\\Temp\\agentdrafter-judge",
}

# A Windows folder the judge can quote on a command line and copy to with scp.
SCRATCH_RX = re.compile(r"^[A-Za-z]:\\[A-Za-z0-9_.\\-]+$")


class SettingsError(Exception):
    pass


@dataclass
class Settings:
    dir: Path | None
    host: str | None
    scratch: str | None
    accoreconsole: str
    source: str  # where the file settings came from, for messages ("" when none)

    def require(self, *names: str) -> None:
        missing = [n for n in names if not getattr(self, n)]
        if missing:
            lines = ["the judge needs these settings:"]
            for n in missing:
                lines.append("  %s (or `%s = ...` in the config file): %s" % (ENV[n], n, HELP[n]))
            lines.append("config file: $%s, else $XDG_CONFIG_HOME/agentdrafter/judge.toml, "
                         "else ~/.config/agentdrafter/judge.toml" % CONFIG_ENV)
            raise SettingsError("\n".join(lines))


def config_path(environ=os.environ) -> Path:
    if environ.get(CONFIG_ENV):
        return Path(environ[CONFIG_ENV]).expanduser()
    base = environ.get("XDG_CONFIG_HOME") or str(Path.home() / ".config")
    return Path(base) / "agentdrafter" / "judge.toml"


def load(environ=os.environ, repo_root: Path | None = None) -> Settings:
    """Settings from the config file, overridden by the environment. Checks that the judge's folder is
    outside the repository, so AutoCAD's readings of private drawings can never land in it."""
    values: dict = {}
    path = config_path(environ)
    source = ""
    if path.is_file():
        import tomllib

        with open(path, "rb") as f:
            try:
                data = tomllib.load(f)
            except tomllib.TOMLDecodeError as e:
                raise SettingsError("the judge's config file is not valid TOML: %s" % e) from None
        unknown = sorted(set(data) - set(ENV))
        if unknown:
            raise SettingsError("unknown keys in the judge's config file: %s (known: %s)"
                                % (", ".join(unknown), ", ".join(ENV)))
        values.update({k: str(v) for k, v in data.items()})
        source = "config file"
    for key, var in ENV.items():
        if environ.get(var):
            values[key] = environ[var]
    d = Path(values["dir"]).expanduser().resolve() if values.get("dir") else None
    if d is not None and repo_root is not None:
        root = repo_root.resolve()
        if d == root or root in d.parents:
            raise SettingsError("%s points inside the repository; the judge's folder holds private drawings "
                                "and must be outside it" % ENV["dir"])
    scratch = values.get("scratch") or None
    if scratch is not None:
        scratch = scratch.rstrip("\\")
        if not SCRATCH_RX.match(scratch):
            raise SettingsError("%s must be a Windows folder such as C:\\Temp\\agentdrafter-judge, made of letters, "
                                "digits, '.', '_', '-' and backslashes (no spaces or quotes)" % ENV["scratch"])
    return Settings(dir=d, host=values.get("host") or None, scratch=scratch,
                    accoreconsole=values.get("accoreconsole") or DEFAULT_ACCORECONSOLE, source=source)

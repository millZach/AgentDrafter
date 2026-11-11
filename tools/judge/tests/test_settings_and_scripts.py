import os
import re
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

JUDGE = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(JUDGE))

from judgelib import autocad, settings  # noqa: E402


class Settings(unittest.TestCase):
    def config(self, t, text):
        p = Path(t, "judge.toml")
        p.write_text(text)
        return {"AGENTDRAFTER_JUDGE_CONFIG": str(p)}

    def test_nothing_private_has_a_default(self):
        with tempfile.TemporaryDirectory() as t:
            s = settings.load({"AGENTDRAFTER_JUDGE_CONFIG": str(Path(t, "none.toml"))})
            self.assertEqual((s.dir, s.host, s.scratch), (None, None, None))
            with self.assertRaises(settings.SettingsError) as e:
                s.require("dir", "host", "scratch")
            for var in ("AGENTDRAFTER_JUDGE_DIR", "AGENTDRAFTER_JUDGE_HOST", "AGENTDRAFTER_JUDGE_SCRATCH"):
                self.assertIn(var, str(e.exception))

    def test_the_environment_wins_over_the_config_file(self):
        with tempfile.TemporaryDirectory() as t:
            env = self.config(t, 'dir = "%s"\nhost = "cad-box"\nscratch = \'C:\\Temp\\judge\\\'\n' % Path(t, "j").as_posix())
            s = settings.load(env)
            self.assertEqual((s.host, s.scratch), ("cad-box", "C:\\Temp\\judge"))
            s = settings.load({**env, "AGENTDRAFTER_JUDGE_HOST": "other-box"})
            self.assertEqual(s.host, "other-box")

    def test_the_judge_folder_may_not_be_inside_the_repository(self):
        with tempfile.TemporaryDirectory() as t:
            env = {"AGENTDRAFTER_JUDGE_CONFIG": str(Path(t, "none.toml")), "AGENTDRAFTER_JUDGE_DIR": str(Path(t, "repo", "x"))}
            with self.assertRaises(settings.SettingsError):
                settings.load(env, repo_root=Path(t, "repo"))
            self.assertIsNotNone(settings.load(env, repo_root=Path(t, "elsewhere")).dir)

    @unittest.skipUnless(shutil.which("git"), "needs git")
    def test_the_judge_folder_may_not_be_inside_any_working_tree_of_the_repository(self):
        with tempfile.TemporaryDirectory() as t:
            main, linked = Path(t, "main"), Path(t, "linked")
            main.mkdir()
            Path(t, "gitconfig").write_text("")
            who = {"GIT_%s_%s" % (r, k): v for r in ("AUTHOR", "COMMITTER")
                   for k, v in (("NAME", "judge test"), ("EMAIL", "judge-test@example.invalid"))}
            env = {**os.environ, **who, "GIT_CONFIG_GLOBAL": str(Path(t, "gitconfig")), "GIT_CONFIG_NOSYSTEM": "1"}
            for args in (["init", "-q"], ["commit", "-q", "--allow-empty", "-m", "first"],
                         ["worktree", "add", "-q", str(linked)]):
                subprocess.run(["git", *args], cwd=main, env=env, check=True, capture_output=True)
            base = {"AGENTDRAFTER_JUDGE_CONFIG": str(Path(t, "none.toml"))}
            # run from the linked worktree, a folder in the main checkout is refused, and the other way round
            for repo_root, other in ((linked, main), (main, linked)):
                with self.assertRaises(settings.SettingsError):
                    settings.load({**base, "AGENTDRAFTER_JUDGE_DIR": str(other / "judge")}, repo_root=repo_root)
            self.assertIsNotNone(settings.load({**base, "AGENTDRAFTER_JUDGE_DIR": str(Path(t, "judge"))},
                                               repo_root=linked).dir)

    def test_a_scratch_folder_the_judge_cannot_quote_is_refused(self):
        with tempfile.TemporaryDirectory() as t:
            base = {"AGENTDRAFTER_JUDGE_CONFIG": str(Path(t, "none.toml"))}
            for bad in ('C:\\My Files\\judge', 'C:\\a"b', "/tmp/judge", "judge"):
                with self.assertRaises(settings.SettingsError):
                    settings.load({**base, "AGENTDRAFTER_JUDGE_SCRATCH": bad})

    def test_unknown_config_keys_are_refused(self):
        with tempfile.TemporaryDirectory() as t:
            with self.assertRaises(settings.SettingsError):
                settings.load(self.config(t, 'hots = "cad-box"\n'))


class Scripts(unittest.TestCase):
    def test_the_lisp_is_pasted_one_form_per_line_without_comments(self):
        forms = autocad.lisp_forms(';; note\n(defun f (a)\n  ; inside\n  (princ "x ; y"))\n(f 1)\n')
        self.assertEqual(forms, ['(defun f (a) (princ "x ; y"))', "(f 1)"])
        with self.assertRaises(ValueError):
            autocad.lisp_forms("(defun f (")

    def test_the_open_script_runs_the_judge_and_quits(self):
        scr = autocad.judge_script((JUDGE / "laptop" / "judge.lsp").read_text())
        lines = scr.split("\r\n")
        self.assertEqual(lines[-3:], ["_.QUIT", "_Y", ""])
        self.assertIn('(adb-judge "clean")', lines)
        self.assertTrue(all(";" not in ln for ln in lines))

    def test_the_recover_script_names_the_drawing_in_the_fixed_folder(self):
        scr = autocad.recover_script("C:/Temp/judge/lanes/judging", "m001")
        self.assertIn("_.RECOVER\r\nC:/Temp/judge/lanes/judging/m001.dwg\r\n", scr)
        self.assertIn('"MODE\\trecovered"', scr)


class Budgets(unittest.TestCase):
    def test_the_ssh_call_outlasts_a_full_wait_before_every_drawing(self):
        class Machine:
            accore = "accoreconsole.exe"

            def __init__(self):
                self.timeouts = []

            def put(self, files, *parts):
                pass

            def win(self, *parts):
                return "\\".join(("C:\\Temp\\judge",) + parts)

            def ps_file(self, script, args, timeout=180):
                self.timeouts.append(timeout)
                return ""

        for n in (1, 2, 13):
            m = Machine()
            autocad._run_driver(m, Path("jobs1.tsv"), "results1.tsv", n)
            # each drawing: the wait for its turn, its time limit, and 10 s to stop it
            self.assertEqual(len(m.timeouts), 1)
            self.assertGreaterEqual(m.timeouts[0], n * (autocad.MAX_WAIT_SEC + autocad.TIMEOUT_SEC + 10))


class Glossary(unittest.TestCase):
    def test_our_own_words_say_object(self):
        # CONTEXT.md says object; literal DXF and AutoCAD names (the ENTITIES section, AcDbEntity) stay
        rx = re.compile(r"(?<![A-Za-z])[Ee]ntit(?:y|ies)")
        texts = {p.name: p.read_text(encoding="utf-8") for p in JUDGE.rglob("*")
                 if p.is_file() and p.suffix in (".py", ".ps1", ".lsp", ".md")}
        context = (JUDGE.parents[1] / "CONTEXT.md").read_text(encoding="utf-8")
        texts["CONTEXT.md: Drawn object"] = context.split("**Drawn object**:", 1)[1].split("\n_Avoid_", 1)[0]
        for name, text in texts.items():
            self.assertIsNone(rx.search(text), name)


class NothingPrivate(unittest.TestCase):
    def test_the_judge_holds_no_home_folders_or_user_paths(self):
        # Settings carry the machine-specific places; the tool's own files name none.
        rx = re.compile(r"/home/[a-z]|/Users/[A-Za-z]|[A-Za-z]:\\{1,2}Users\\{1,2}(?!\\*<you>)", re.I)
        for p in JUDGE.rglob("*"):
            if p.is_file() and p.suffix in (".py", ".ps1", ".lsp", ".md", ".toml"):
                self.assertIsNone(rx.search(p.read_text(encoding="utf-8")), p.name)


if __name__ == "__main__":
    unittest.main()

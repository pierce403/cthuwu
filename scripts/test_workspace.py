import importlib.util
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
from types import SimpleNamespace

spec = importlib.util.spec_from_file_location("workspace", Path(__file__).with_name("workspace.py"))
w = importlib.util.module_from_spec(spec)
spec.loader.exec_module(w)

class WorkspaceTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        w.init(self.root)

    def test_rebuild_and_deletion_remove_stale_retrieval(self):
        source = self.root / "knowledge/routine.md"
        source.write_text("# Walking\nSource: personal experiment\nA morning walk helps build a routine.\n")
        def embedding(texts, _model):
            return [[1.0, 0.0] for _ in texts]
        self.assertEqual(w.index(self.root, "test", embedding)["updated"], 1)
        self.assertEqual(w.index(self.root, "test", embedding)["updated"], 0)
        result = w.search(self.root, "exercise", "test", embedding=embedding)
        self.assertEqual(result[0]["path"], "knowledge/routine.md")
        (self.root / ".knowledge-index/index.sqlite").unlink()
        w.index(self.root, "test", embedding)
        self.assertEqual(w.search(self.root, "exercise", "test", embedding=embedding), result)
        source.write_text("Corrected information\n")
        self.assertEqual(w.search(self.root, "exercise", "test", embedding=embedding), [])
        source.unlink()
        self.assertEqual(w.search(self.root, "walk", ""), [])
        self.assertEqual(w.index(self.root, "test", embedding)["removed"], 1)

    def test_upstream_cursor_deduplicates_without_checking_out_source(self):
        repository = self.root / "source"
        repository.mkdir()
        commands = []
        def run(command, **_kwargs):
            commands.append(command)
            if "remote" in command:
                value = "https://github.com/example/upstream.git"
            elif "rev-parse" in command:
                value = "a" * 40
            elif "log" in command:
                value = "aaaaaaa Improve local memory retrieval"
            else:
                value = ""
            return SimpleNamespace(stdout=value)
        with patch.object(w.subprocess, "run", side_effect=run):
            first = w.upstream(self.root, "source")
            second = w.upstream(self.root, "source")
        self.assertTrue(first["changed"])
        self.assertFalse(second["changed"])
        note = (self.root / first["note"]).read_text()
        self.assertIn("Source: https://github.com/example/upstream.git", note)
        self.assertIn("Observed:", note)
        self.assertFalse(any(action in command for command in commands for action in ("checkout", "merge", "pull", "reset")))

    def test_private_scopes_and_symlinks_are_excluded(self):
        (self.root / "knowledge/private.md").write_text("private: true\nsecretgoal")
        (self.root / "acolytes").mkdir()
        (self.root / "acolytes/alice.md").write_text("secretgoal")
        w.index(self.root, "")
        self.assertEqual(w.search(self.root, "secretgoal", ""), [])
        (self.root / "knowledge/escape.md").symlink_to(self.root / "acolytes/alice.md")
        with self.assertRaises(ValueError):
            w.index(self.root, "")

    def test_skill_revision_retirement_and_no_overwrite_on_init(self):
        w.skill(self.root, "weekly-plan", "Make a practical plan", "Choose one step and verify progress.")
        w.skill(self.root, "weekly-plan", "Make a practical plan", "Ask about capacity before choosing a step.")
        self.assertEqual(len(list((self.root / "skills/weekly-plan/.revisions").glob("*.md"))), 1)
        w.index(self.root, "")
        self.assertTrue(w.search(self.root, "capacity", ""))
        w.skill(self.root, "weekly-plan", "Retired plan", "Use the replacement skill.", retire=True)
        self.assertEqual(w.search(self.root, "capacity", ""), [])
        mission = self.root / "MISSION.md"
        mission.write_text("My curated mission")
        w.init(self.root)
        self.assertEqual(mission.read_text(), "My curated mission")

if __name__ == "__main__":
    unittest.main()

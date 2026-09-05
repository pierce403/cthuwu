#!/usr/bin/env python3
"""Offline Git topology, activation, cancellation, and storage-boundary checks."""
import importlib.util
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
import time
import unittest
from unittest import mock

sys.dont_write_bytecode = True
SPEC = importlib.util.spec_from_file_location("tentacle_code", Path(__file__).with_name("code.py"))
code = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(code)


class FixtureWorkspace(code.CodeWorkspace):
    """The local transport/build seam exists only in this test module, never CLI."""
    def __init__(self, root, upstream):
        super().__init__(root)
        self.fixture_upstream = str(upstream)
        self.env["GIT_ALLOW_PROTOCOL"] = "file"
        self.fail_build = False

    def git(self, *args, **kwargs):
        args = list(args)
        if "fetch" in args:
            args = [self.fixture_upstream if item == self.upstream else item for item in args]
        return super().git("-c", "protocol.file.allow=always", *args, **kwargs)

    def build_release(self, commit):
        if self.fail_build:
            raise ValueError("fixture build failed")
        directory = code.contained(self.root, f"releases/{commit}")
        if directory.exists():
            manifest = code.read_json(directory / "manifest.json")
            self.validate_release(manifest)
            return manifest
        (directory / "agent/dist").mkdir(parents=True)
        (directory / "agent/node_modules").mkdir()
        (directory / "agent/package.json").write_text('{"type":"module"}\n')
        binary, sidecar = directory / "uwubot", directory / "agent/dist/index.js"
        binary.write_text("#!/bin/sh\nexit 0\n")
        binary.chmod(0o700)
        sidecar.write_text("// " + commit + "\n")
        manifest = {"version": 1, "commit": commit, "built_at": code.timestamp(),
                    "binary": f"releases/{commit}/uwubot",
                    "sidecar": f"releases/{commit}/agent/dist/index.js",
                    "binary_sha256": code.hashlib.sha256(binary.read_bytes()).hexdigest(),
                    "sidecar_sha256": code.hashlib.sha256(sidecar.read_bytes()).hexdigest()}
        code.atomic(directory / "manifest.json", json.dumps(manifest))
        self.validate_release(manifest)
        return manifest


class SourceTests(unittest.TestCase):
    def setUp(self):
        scratch = Path(__file__).resolve().parents[1] / "tmp"
        scratch.mkdir(exist_ok=True)
        self.temporary = tempfile.TemporaryDirectory(prefix="source-test-", dir=scratch)
        self.base = Path(self.temporary.name)
        self.upstream = self.base / "upstream"
        self.upstream.mkdir()
        self.git_env = {"PATH": os.defpath, "HOME": str(self.base),
                        "GIT_CONFIG_NOSYSTEM": "1", "GIT_CONFIG_GLOBAL": "/dev/null",
                        "GIT_AUTHOR_NAME": "Test", "GIT_AUTHOR_EMAIL": "test@localhost",
                        "GIT_COMMITTER_NAME": "Test", "GIT_COMMITTER_EMAIL": "test@localhost"}
        self.git("init", "--initial-branch=main")
        self.initial = self.commit("notes.md", "initial\n", "Initial life coaching code")
        self.workspace = FixtureWorkspace(self.base / "workspace", self.upstream)

    def tearDown(self):
        self.temporary.cleanup()

    def git(self, *args, cwd=None):
        return subprocess.run(["git", "-c", "core.hooksPath=/dev/null", *args],
                              cwd=cwd or self.upstream, env=self.git_env,
                              check=True, text=True, capture_output=True).stdout.strip()

    def commit(self, path, body, reason, cwd=None):
        directory = cwd or self.upstream
        (directory / path).write_text(body)
        self.git("add", path, cwd=directory)
        self.git("commit", "-m", reason, cwd=directory)
        return self.git("rev-parse", "HEAD", cwd=directory)

    def test_init_preserves_markdown_and_independent_branch(self):
        body = code.DEFAULT_CODE + "\nOperator note: patient coaching first.\n"
        self.workspace.code_path.write_text(body)
        result = self.workspace.init()
        self.assertEqual(result["source_commit"], self.initial)
        self.assertEqual(result["local_branch"], "tentacle")
        self.assertIn("patient coaching first", self.workspace.code_path.read_text())
        self.assertEqual(code.DEFAULT_CODE, (Path(__file__).parents[1] / "cthuwu/agent-files/CODE.md").read_text())

    def test_noop_update_installs_and_fast_forward_installs_new_source(self):
        first = self.workspace.update()
        self.assertEqual(first["installed_commit"], self.initial)
        self.assertTrue(first["restart_required"])
        candidate = self.commit("tips.md", "Small achievable goals.\n", "Improve coaching prompts")
        updated = self.workspace.update()
        self.assertEqual(updated["source_commit"], candidate)
        self.assertEqual(updated["installed_commit"], candidate)
        self.assertTrue((self.workspace.root / "releases" / self.initial / "uwubot").exists())
        self.assertIsNone(updated["pending_operation"])

    def test_failed_build_preserves_source_and_active_release(self):
        self.workspace.update()
        active = (self.workspace.root / "releases/active.json").read_bytes()
        self.commit("tips.md", "new\n", "New upstream feature")
        self.workspace.fail_build = True
        with self.assertRaisesRegex(ValueError, "fixture build failed"):
            self.workspace.update()
        self.assertEqual(self.workspace.status()["source_commit"], self.initial)
        self.assertEqual((self.workspace.root / "releases/active.json").read_bytes(), active)

    def test_force_update_installs_main_and_preserves_dirty_divergent_source(self):
        self.workspace.init()
        local = self.commit("local.md", "tailored coaching\n", "Local improvement", cwd=self.workspace.repo)
        (self.workspace.repo / "notes.md").write_text("unsaved local work\n")
        tip = self.commit("tips.md", "main improvement\n", "Main update")
        self.workspace.code_path.write_text(self.workspace.code_path.read_text().replace("branch: main", "branch: missing-branch"))
        result = self.workspace.force_update()
        self.assertEqual(result["installed_commit"], tip)
        self.assertEqual(result["source_commit"], local)
        self.assertEqual((self.workspace.repo / "notes.md").read_text(), "unsaved local work\n")
        self.assertTrue(result["restart_required"])
        self.assertIn("NOT in the installed release", result["divergence"])
        self.assertIn("force-installed main", self.workspace.code_path.read_text())

    def test_force_update_fresh_workspace_uses_main_and_failed_build_preserves_release(self):
        self.workspace.code_path.write_text(code.DEFAULT_CODE.replace("branch: main", "branch: missing-branch"))
        result = self.workspace.force_update()
        self.assertEqual(result["installed_commit"], self.initial)
        active = (self.workspace.root / "releases/active.json").read_bytes()
        tip = self.commit("tips.md", "new\n", "Main update")
        self.workspace.fail_build = True
        with self.assertRaisesRegex(ValueError, "fixture build failed"):
            self.workspace.force_update()
        self.assertEqual(self.workspace.status()["source_commit"], self.initial)
        self.assertEqual((self.workspace.root / "releases/active.json").read_bytes(), active)
        self.workspace.fail_build = False
        result = self.workspace.force_update()
        self.assertEqual(result["installed_commit"], tip)
        self.assertEqual(result["source_commit"], tip)

    def test_dirty_checkout_and_executable_git_config_are_refused(self):
        self.workspace.init()
        (self.workspace.repo / "notes.md").write_text("unsaved idea\n")
        with self.assertRaisesRegex(ValueError, "uncommitted"):
            self.workspace.update()
        self.assertEqual((self.workspace.repo / "notes.md").read_text(), "unsaved idea\n")
        self.git("config", "filter.untrusted.smudge", "touch /tmp/should-not-exist", cwd=self.workspace.repo)
        with self.assertRaisesRegex(ValueError, "unsupported configuration"):
            self.workspace.status()

    def test_divergence_selective_adoption_override_and_repeated_adoption(self):
        self.workspace.init()
        local = self.commit("local.md", "Better follow-up questions.\n", "Keep tailored coaching improvements", cwd=self.workspace.repo)
        selected = self.commit("tips.md", "A useful new capability.\n", "Add useful coaching tip")
        deferred = self.commit("style.md", "Noisy flourish.\n", "Add optional dramatic flourish")
        reviewed = self.workspace.update()
        self.assertEqual(reviewed["action"], "review_required")
        self.assertEqual(reviewed["source_commit"], local)
        self.workspace.defer([selected], "Initially unnecessary for my acolytes")
        result = self.workspace.accept([selected], "Operator override: this capability helps our current goals")
        self.assertEqual(result["adopted"], [selected])
        self.assertTrue((self.workspace.repo / "local.md").exists())
        self.assertTrue((self.workspace.repo / "tips.md").exists())
        self.assertFalse((self.workspace.repo / "style.md").exists())
        again = self.workspace.review()
        self.assertNotIn(selected, again["upstream_commits"])
        self.assertIn(deferred, again["upstream_commits"])
        self.assertEqual(self.workspace.accept([selected], "Already useful")["action"], "already_adopted")
        self.assertIn("Keep tailored coaching improvements", self.workspace.code_path.read_text())
        self.assertIn("Operator override", self.workspace.code_path.read_text())

    def test_conflicting_selection_preserves_every_local_change(self):
        self.workspace.update()
        local = self.commit("notes.md", "local approach\n", "Prefer the local coaching approach", cwd=self.workspace.repo)
        valid = self.commit("useful.md", "first change\n", "Useful upstream change")
        conflict = self.commit("notes.md", "different upstream approach\n", "Conflicting upstream approach")
        self.workspace.review()
        old_active = self.workspace.active()
        with self.assertRaisesRegex(ValueError, "CONFLICT"):
            self.workspace.accept([valid, conflict], "Try the upstream feature set")
        self.assertEqual(self.workspace.status()["source_commit"], local)
        self.assertEqual(self.workspace.active(), old_active)
        self.assertFalse((self.workspace.repo / "useful.md").exists())
        self.assertEqual(len(self.git("worktree", "list", "--porcelain", cwd=self.workspace.repo).split("worktree ")) - 1, 1)

    def test_intent_survives_activation_failure_and_install_recovers_reason(self):
        self.workspace.update()
        self.commit("local.md", "local\n", "Local coaching improvement", cwd=self.workspace.repo)
        selected = self.commit("tips.md", "new\n", "Useful upstream addition")
        self.workspace.review()
        previous_active = self.workspace.active()["commit"]
        with mock.patch.object(self.workspace, "activate", side_effect=OSError("fixture disk full")):
            with self.assertRaisesRegex(OSError, "disk full"):
                self.workspace.accept([selected], "Operator specifically requested this useful capability")
        status = self.workspace.status()
        self.assertEqual(status["installed_commit"], previous_active)
        self.assertEqual(status["pending_operation"]["commits"], [selected])
        self.assertEqual(status["source_commit"], status["pending_operation"]["candidate"])
        self.workspace.install()
        state = self.workspace.state()
        self.assertNotIn("pending", state)
        self.assertTrue(any(item["commit"] == selected and "Operator specifically" in item["reason"] for item in state["decisions"]))

    def test_cached_invalid_release_cannot_replace_active_pointer(self):
        self.workspace.update()
        old = (self.workspace.root / "releases/active.json").read_bytes()
        candidate = self.commit("new.md", "new\n", "New feature")
        self.workspace.review()
        manifest = self.workspace.build_release(candidate)
        (self.workspace.root / manifest["binary"]).chmod(0o600)
        with self.assertRaisesRegex(ValueError, "not executable"):
            self.workspace.update()
        self.assertEqual((self.workspace.root / "releases/active.json").read_bytes(), old)
        self.assertEqual(self.workspace.status()["source_commit"], self.initial)

    def test_prime_change_during_build_cancels_promotion(self):
        self.workspace.update()
        self.commit("new.md", "new\n", "New upstream feature")
        builder = self.workspace.build_release
        def changed_configuration(commit):
            release = builder(commit)
            self.workspace.code_path.write_text(self.workspace.code_path.read_text().replace("branch: main", "branch: next"))
            return release
        with mock.patch.object(self.workspace, "build_release", side_effect=changed_configuration):
            with self.assertRaisesRegex(ValueError, "configuration changed"):
                self.workspace.update()
        self.assertEqual(self.workspace.status()["source_commit"], self.initial)
        self.assertEqual(self.workspace.active()["commit"], self.initial)
        self.assertIn("branch: next", self.workspace.code_path.read_text())

    def test_config_rejects_non_github_credentials_and_path_symlinks(self):
        for url in ("https://user:secret@github.com/a/b", "file:///tmp/a", "https://github.com/a/b?token=secret", "https://example.com/a/b"):
            self.workspace.code_path.write_text(code.DEFAULT_CODE.replace("https://github.com/pierce403/cthuwu.git", url))
            with self.assertRaisesRegex(ValueError, "credential-free HTTPS GitHub"):
                self.workspace.configuration()
        self.workspace.code_path.unlink()
        self.workspace.code_path.symlink_to(self.base / "outside.md")
        with self.assertRaisesRegex(ValueError, "symlink"):
            code.CodeWorkspace(self.workspace.root)

    def test_environment_storage_stays_inside_workspace(self):
        readonly = self.base / "preinstalled-bin"
        readonly.mkdir()
        with mock.patch.dict(os.environ, {"UWUBOT_READONLY_TOOL_PATH": str(readonly), "VENICE_API_KEY": "never-forward-this"}):
            env = code.workspace_environment(self.workspace.root)
        for key, value in env.items():
            if key.endswith(("HOME", "DIR", "PREFIX", "CACHE", "REPOSITORY", "LOGS")) or key in ("TMP", "TEMP", "OLLAMA_MODELS", "npm_config_store_dir"):
                self.assertTrue(Path(value).is_relative_to(self.workspace.root), (key, value))
        self.assertIn(str(readonly), env["PATH"].split(os.pathsep))
        self.assertNotIn("VENICE_API_KEY", env)
        self.assertNotIn("never-forward-this", str(env))

    def test_outer_cancellation_kills_helper_children_in_the_same_group(self):
        ready = self.workspace.root / "tmp/child-ready"
        continued = self.workspace.root / "tmp/child-continued"
        grandchild = "import pathlib,time; pathlib.Path(%r).write_text('ready'); time.sleep(2); pathlib.Path(%r).write_text('escaped')" % (str(ready), str(continued))
        child = "import subprocess,sys,time; subprocess.Popen([sys.executable,'-c',%r]); time.sleep(5)" % grandchild
        driver = "import importlib.util,sys; s=importlib.util.spec_from_file_location('tc',sys.argv[1]); m=importlib.util.module_from_spec(s); s.loader.exec_module(m); m.CodeWorkspace(sys.argv[2]).run([sys.executable,'-c',sys.argv[3]])"
        process = subprocess.Popen([sys.executable, "-B", "-c", driver, str(Path(code.__file__).resolve()), str(self.workspace.root), child],
                                   start_new_session=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        try:
            deadline = time.monotonic() + 4
            while not ready.exists() and time.monotonic() < deadline:
                time.sleep(0.02)
            self.assertTrue(ready.exists())
            os.killpg(process.pid, signal.SIGKILL)
            process.wait(timeout=3)
            time.sleep(2.1)
            self.assertFalse(continued.exists())
        finally:
            if process.poll() is None:
                os.killpg(process.pid, signal.SIGKILL)
                process.wait()


if __name__ == "__main__":
    unittest.main()

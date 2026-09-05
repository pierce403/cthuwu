#!/usr/bin/env python3
"""No-network checks for installed-release selection and workspace-owned tool storage."""

import hashlib
import importlib.util
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest import mock

SCRIPT = Path(__file__).with_name("release-entrypoint.py")
spec = importlib.util.spec_from_file_location("release_entrypoint", SCRIPT)
release = importlib.util.module_from_spec(spec)
spec.loader.exec_module(release)


class ReleaseTests(unittest.TestCase):
    def setUp(self):
        temporary = SCRIPT.parent.parent / "tmp"
        temporary.mkdir(exist_ok=True)
        self.temporary = tempfile.TemporaryDirectory(prefix="release-test-", dir=temporary)
        self.addCleanup(self.temporary.cleanup)
        self.workspace = Path(self.temporary.name)
        self.commit = "a" * 40

    def installed(self):
        directory = self.workspace / "releases" / self.commit
        (directory / "agent/dist").mkdir(parents=True)
        (directory / "agent/node_modules").mkdir()
        (directory / "agent/package.json").write_text('{"type":"module"}')
        binary, sidecar = directory / "uwubot", directory / "agent/dist/index.js"
        binary.write_bytes(b"#!/bin/sh\nexit 0\n")
        binary.chmod(0o700)
        sidecar.write_text("export {};\n")
        core = {"version": 1, "commit": self.commit,
                "binary": f"releases/{self.commit}/uwubot",
                "sidecar": f"releases/{self.commit}/agent/dist/index.js"}
        manifest = dict(core, built_at="2026-09-04T00:00:00Z",
                        binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(),
                        sidecar_sha256=hashlib.sha256(sidecar.read_bytes()).hexdigest())
        (directory / "manifest.json").write_text(json.dumps(manifest))
        active = dict(core, activated_at="2026-09-04T01:00:00+00:00")
        (directory.parent / "active.json").write_text(json.dumps(active))
        return binary, sidecar, active

    def test_release_pairs_verified_binary_and_sidecar(self):
        self.assertIsNone(release.select_release(self.workspace))
        binary, sidecar, _ = self.installed()
        self.assertEqual((binary, sidecar, self.commit), release.select_release(self.workspace))
        sidecar.write_text("changed after installation")
        with self.assertRaisesRegex(release.ReleaseError, "does not match"):
            release.select_release(self.workspace)

    def test_corrupt_pointer_does_not_fall_back(self):
        self.installed()
        (self.workspace / "releases/active.json").write_text("not json")
        with self.assertRaises(release.ReleaseError):
            release.select_release(self.workspace)

    def test_rejects_cross_release_paths_and_symlinks(self):
        binary, _, active = self.installed()
        pointer = self.workspace / "releases/active.json"
        active["binary"] = "../../usr/local/bin/uwubot"
        pointer.write_text(json.dumps(active))
        with self.assertRaisesRegex(release.ReleaseError, "disagree"):
            release.select_release(self.workspace)
        active["binary"] = f"releases/{self.commit}/uwubot"
        pointer.write_text(json.dumps(active))
        replacement = self.workspace / "replacement"
        binary.rename(replacement)
        binary.symlink_to(replacement)
        with self.assertRaisesRegex(release.ReleaseError, "symbolic links"):
            release.select_release(self.workspace)

    def test_dangling_release_pointer_is_an_error(self):
        (self.workspace / "releases").mkdir()
        (self.workspace / "releases/active.json").symlink_to(self.workspace / "absent")
        with self.assertRaisesRegex(release.ReleaseError, "symbolic links"):
            release.select_release(self.workspace)

    def test_workspace_environment_replaces_host_storage_but_keeps_private_data(self):
        environment = release.workspace_environment(self.workspace, {
            "HOME": "/host/home", "TMPDIR": "/tmp", "npm_config_prefix": "/usr/local",
            "UWUBOT_DATA_DIR": "/private/tentacle", "UWUBOT_MODEL_API_KEY": "private-key"})
        for key in ("HOME", "TMPDIR", "TMP", "TEMP", "CARGO_HOME", "RUSTUP_HOME",
                    "npm_config_prefix", "npm_config_cache", "PNPM_STORE_DIR", "HOMEBREW_PREFIX"):
            self.assertTrue(Path(environment[key]).is_relative_to(self.workspace), key)
        for key in ("UV_CACHE_DIR", "UV_PYTHON_INSTALL_DIR", "UV_TOOL_DIR", "UV_TOOL_BIN_DIR",
                    "OLLAMA_MODELS", "npm_config_store_dir"):
            self.assertTrue(Path(environment[key]).is_relative_to(self.workspace), key)
        self.assertEqual("/private/tentacle", environment["UWUBOT_DATA_DIR"])
        self.assertEqual("private-key", environment["UWUBOT_MODEL_API_KEY"])
        self.assertEqual("true", environment["PIP_REQUIRE_VIRTUALENV"])
        self.assertEqual(str(self.workspace / "tools/bin"), environment["PATH"].split(":")[0])

    def test_workspace_cache_symlink_is_rejected_before_use(self):
        (self.workspace / "tools").mkdir()
        (self.workspace / "tools/npm-cache").symlink_to(self.workspace)
        with self.assertRaisesRegex(release.ReleaseError, "symbolic links"):
            release.workspace_environment(self.workspace)

    def test_cli_workspace_precedes_environment_and_stops_at_terminator(self):
        with mock.patch.dict(os.environ, {"UWUBOT_OPERATOR_ROOT": "/from-env"}):
            self.assertEqual(Path("/from-cli"), release.configured_workspace([
                "--operator-root=/from-cli", "--", "--operator-root=/ignored"]))

    def test_bootstrap_copies_source_without_installing_in_source_checkout(self):
        source = self.workspace / "source/agent"
        (source / "src").mkdir(parents=True)
        for name in ("package.json", "package-lock.json", "tsconfig.json", "tsconfig.build.json"):
            (source / name).write_text("{}")
        (source / "src/index.ts").write_text("export {};")
        release.workspace_environment(self.workspace)
        release.stage_agent(self.workspace, source)
        target = self.workspace / "tools/bootstrap-agent"
        self.assertEqual("export {};", (target / "src/index.ts").read_text())
        (target / "src/removed.ts").write_text("stale source")
        (target / "dist").mkdir()
        (target / "dist/removed.js").write_text("stale build")
        release.stage_agent(self.workspace, source)
        self.assertFalse((target / "src/removed.ts").exists())
        self.assertFalse((target / "dist").exists())
        self.assertFalse((source / "node_modules").exists())

    def test_container_exec_receipt_uses_matching_release_without_restart_loop(self):
        binary, sidecar, _ = self.installed()
        with mock.patch.dict(os.environ, {"UWUBOT_OPERATOR_ROOT": str(self.workspace)}), \
                mock.patch.object(os, "execvpe") as execute:
            release.main(["--skip-awakening"])
        executable, arguments, environment = execute.call_args.args
        self.assertEqual(str(binary), executable)
        self.assertEqual([str(binary), "--skip-awakening"], arguments)
        self.assertEqual(str(sidecar), environment["UWUBOT_SIDECAR"])
        self.assertEqual(self.commit, environment["UWUBOT_RUNNING_SOURCE"])


if __name__ == "__main__":
    unittest.main()

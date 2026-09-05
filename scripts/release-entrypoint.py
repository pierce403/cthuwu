#!/usr/bin/env python3
"""Workspace-owned child environment and deliberate installed-release startup."""

import datetime
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import stat
import sys


class ReleaseError(ValueError):
    pass


def real_path(path, directory=False, create=False):
    """Reject links before creating or using any component below the filesystem root."""
    path = Path(os.path.abspath(path))
    if "\n" in str(path) or "\r" in str(path):
        raise ReleaseError("workspace and release paths must not contain line breaks")
    current = Path(path.anchor)
    for component in path.parts[1:]:
        current /= component
        if create and not current.exists() and not current.is_symlink():
            current.mkdir(mode=0o700)
        try:
            mode = current.lstat().st_mode
        except FileNotFoundError as error:
            raise ReleaseError(f"required path is missing: {current}") from error
        if stat.S_ISLNK(mode):
            raise ReleaseError(f"symbolic links are not allowed in workspace paths: {current}")
        if current != path and not stat.S_ISDIR(mode):
            raise ReleaseError(f"workspace parent is not a directory: {current}")
    mode = path.stat().st_mode
    if directory and not stat.S_ISDIR(mode):
        raise ReleaseError(f"required directory is missing: {path}")
    if not directory and not stat.S_ISREG(mode):
        raise ReleaseError(f"required regular file is missing: {path}")
    return path


def workspace_environment(workspace, inherited=None):
    workspace = real_path(workspace, directory=True, create=True)
    environment = dict(os.environ if inherited is None else inherited)
    paths = {
        "HOME": "tools/home",
        "TMPDIR": "tmp", "TMP": "tmp", "TEMP": "tmp",
        "VIRTUAL_ENV": "tools/venv", "PIP_CACHE_DIR": "tools/pip",
        "PIP_PREFIX": "tools/venv",
        "npm_config_prefix": "tools/npm", "npm_config_cache": "tools/npm-cache",
        "NPM_CONFIG_PREFIX": "tools/npm", "NPM_CONFIG_CACHE": "tools/npm-cache",
        "PNPM_HOME": "tools/pnpm", "PNPM_STORE_DIR": "tools/pnpm-store",
        "npm_config_store_dir": "tools/pnpm-store",
        "UV_CACHE_DIR": "tools/uv-cache", "UV_PYTHON_INSTALL_DIR": "tools/uv-python",
        "UV_TOOL_DIR": "tools/uv-tools", "UV_TOOL_BIN_DIR": "tools/bin",
        "OLLAMA_MODELS": "tools/ollama",
        "HF_HOME": "tools/huggingface", "PYTHONUSERBASE": "tools/python",
        "PYTHONPYCACHEPREFIX": "tools/python-cache",
        "GOPATH": "tools/go", "GOBIN": "tools/go/bin",
        "GOCACHE": "tools/go-cache", "GOMODCACHE": "tools/go-mod",
        "GEM_HOME": "tools/gems", "BUNDLE_PATH": "tools/bundle",
        "CARGO_HOME": "tools/cargo", "RUSTUP_HOME": "tools/rustup",
        "HOMEBREW_PREFIX": "tools/brew", "HOMEBREW_REPOSITORY": "tools/brew",
        "HOMEBREW_CACHE": "tools/brew-cache", "HOMEBREW_LOGS": "tools/brew-logs",
        "XDG_CONFIG_HOME": "tools/xdg/config", "XDG_CACHE_HOME": "tools/xdg/cache",
        "XDG_DATA_HOME": "tools/xdg/data", "XDG_STATE_HOME": "tools/xdg/state",
        "XDG_RUNTIME_DIR": "tools/xdg/runtime",
    }
    for key, relative in paths.items():
        environment[key] = str(real_path(workspace / relative, directory=True, create=True))
    binaries = ["tools/bin", "tools/venv/bin", "tools/pnpm", "tools/cargo/bin",
                "tools/npm/bin", "tools/brew/bin", "tools/brew/sbin",
                "tools/go/bin", "tools/gems/bin", "tools/python/bin"]
    for relative in [*binaries, "tools/build"]:
        real_path(workspace / relative, directory=True, create=True)
    readonly_paths = os.pathsep.join(
        value for value in environment.get("UWUBOT_READONLY_TOOL_PATH", "").split(os.pathsep)
        if value and Path(value).is_absolute() and Path(value).is_dir())
    environment["UWUBOT_READONLY_TOOL_PATH"] = readonly_paths
    environment["PATH"] = os.pathsep.join(str(workspace / p) for p in binaries)
    if readonly_paths:
        environment["PATH"] += ":" + readonly_paths
    environment["PATH"] += ":/usr/local/bin:/usr/bin:/bin"
    environment["PIP_REQUIRE_VIRTUALENV"] = "true"
    environment["PYTHONDONTWRITEBYTECODE"] = "1"
    environment["UWUBOT_OPERATOR_ROOT"] = str(workspace)
    return environment


def stage_agent(workspace, source):
    workspace = real_path(workspace, directory=True)
    source = real_path(source, directory=True)
    target = real_path(workspace / "tools/bootstrap-agent", directory=True, create=True)
    for name in ("package.json", "package-lock.json", "tsconfig.json", "tsconfig.build.json"):
        original = real_path(source / name)
        destination = target / name
        if destination.exists() or destination.is_symlink():
            real_path(destination)
        shutil.copyfile(original, destination)
    for name in ("src", "dist"):
        destination = target / name
        if destination.exists() or destination.is_symlink():
            real_path(destination, directory=True)
            shutil.rmtree(destination)
    original = real_path(source / "src", directory=True)
    for parent, directories, files in os.walk(original):
        for name in directories + files:
            if (Path(parent) / name).is_symlink():
                raise ReleaseError("bootstrap sidecar source may not contain symbolic links")
    shutil.copytree(original, target / "src")


def object_pairs(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ReleaseError("release metadata contains a duplicate field")
        result[key] = value
    return result


def read_metadata(path):
    path = real_path(path)
    if path.stat().st_size > 32768:
        raise ReleaseError("release metadata exceeds 32 KiB")
    try:
        data = json.loads(path.read_text(encoding="utf-8"), object_pairs_hook=object_pairs)
    except (UnicodeError, json.JSONDecodeError) as error:
        raise ReleaseError("release metadata is not valid UTF-8 JSON") from error
    if not isinstance(data, dict) or type(data.get("version")) is not int or data["version"] != 1:
        raise ReleaseError("release metadata requires version 1")
    return data


def timestamp(value):
    if not isinstance(value, str):
        raise ReleaseError("release metadata requires a timezone-qualified timestamp")
    try:
        parsed = datetime.datetime.fromisoformat(value.replace("Z", "+00:00"))
    except ValueError as error:
        raise ReleaseError("release metadata timestamp is invalid") from error
    if parsed.tzinfo is None:
        raise ReleaseError("release metadata timestamp requires a timezone")


def digest(path):
    hasher = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            hasher.update(chunk)
    return hasher.hexdigest()


def select_release(workspace):
    workspace = real_path(workspace, directory=True)
    releases = workspace / "releases"
    if not releases.exists() and not releases.is_symlink():
        return None
    real_path(releases, directory=True)
    pointer = releases / "active.json"
    if not pointer.exists() and not pointer.is_symlink():
        return None
    active = read_metadata(pointer)
    commit = active.get("commit")
    if not isinstance(commit, str) or not re.fullmatch(r"[0-9a-f]{40}", commit):
        raise ReleaseError("active release commit must be a full lowercase Git commit hash")
    timestamp(active.get("activated_at"))
    expected = {"version": 1, "commit": commit,
                "binary": f"releases/{commit}/uwubot",
                "sidecar": f"releases/{commit}/agent/dist/index.js"}
    manifest = read_metadata(releases / commit / "manifest.json")
    for key, value in expected.items():
        if active.get(key) != value or manifest.get(key) != value:
            raise ReleaseError(f"active release and manifest disagree on {key}")
    timestamp(manifest.get("built_at"))
    binary = real_path(workspace / expected["binary"])
    sidecar = real_path(workspace / expected["sidecar"])
    real_path(sidecar.parent.parent / "package.json")
    real_path(sidecar.parent.parent / "node_modules", directory=True)
    if not os.access(binary, os.X_OK):
        raise ReleaseError("installed uwubot binary is not executable")
    for label, path in (("binary", binary), ("sidecar", sidecar)):
        if manifest.get(label + "_sha256") != digest(path):
            raise ReleaseError(f"installed {label} does not match its release manifest")
    return binary, sidecar, commit


def configured_workspace(arguments):
    workspace = os.environ.get("UWUBOT_OPERATOR_ROOT", "/workspace")
    index = 0
    while index < len(arguments):
        argument = arguments[index]
        if argument == "--":
            break
        if argument == "--operator-root":
            index += 1
            if index >= len(arguments):
                raise ReleaseError("--operator-root requires a value")
            workspace = arguments[index]
        elif argument.startswith("--operator-root="):
            workspace = argument.split("=", 1)[1]
        index += 1
    if not workspace:
        raise ReleaseError("the operator workspace may not be empty")
    return Path(os.path.abspath(workspace))


def main(arguments):
    if arguments and arguments[0] == "--stage-agent":
        if len(arguments) != 3:
            raise ReleaseError("--stage-agent requires a workspace and sidecar source")
        stage_agent(arguments[1], arguments[2])
        return 0
    if arguments and arguments[0] == "--select":
        if len(arguments) != 2:
            raise ReleaseError("--select requires one workspace")
        selected = select_release(arguments[1])
        if selected is None:
            return 3
        print("\n".join(str(value) for value in selected))
        return 0
    if arguments and arguments[0] == "--workspace-env":
        if len(arguments) < 3:
            raise ReleaseError("--workspace-env requires a workspace and command")
        environment = workspace_environment(arguments[1])
        os.execvpe(arguments[2], arguments[2:], environment)
    workspace = configured_workspace(arguments)
    environment = workspace_environment(workspace)
    selected = select_release(workspace)
    binary = "/usr/local/bin/uwubot"
    environment.pop("UWUBOT_RUNNING_SOURCE", None)
    if selected:
        binary, sidecar, commit = selected
        if any(a == "--sidecar" or a.startswith("--sidecar=") for a in arguments):
            raise ReleaseError("an installed release owns its matching sidecar; remove --sidecar")
        environment["UWUBOT_SIDECAR"] = str(sidecar)
        environment["UWUBOT_RUNNING_SOURCE"] = commit
        print(f"uwubot: starting installed source {commit[:12]}", file=sys.stderr)
    os.execvpe(str(binary), [str(binary), *arguments], environment)


if __name__ == "__main__":
    try:
        sys.exit(main(sys.argv[1:]))
    except (ReleaseError, OSError) as error:
        print(f"uwubot release startup failed: {error}. Repair the workspace release metadata or deliberately remove releases/active.json to use the shipped runtime.", file=sys.stderr)
        sys.exit(1)

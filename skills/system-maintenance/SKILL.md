---
name: system-maintenance
description: Troubleshoot, debug, inspect, modify code, run tests, and redeploy this Tentacle's checkout.
---

# System maintenance

Use this skill only for an authenticated operator request to troubleshoot, debug, diagnose, update,
validate, edit code, or repair this Tentacle's own Cthuwu checkout.

1. Read this file before calling `repository_maintenance` or making source code edits.
2. For troubleshooting and diagnostics, start with `{"operation":"status"}` (or natural phrases like
   "troubleshoot yourself", "debug yourself", "inspect repository"). Report the canonical repository root,
   HEAD, branch, tracked ref, dirty entries, canonical/fork topology, ahead/behind counts, sanitized
   remotes, and Git/`gh` availability from the receipt.
3. Treat dirty paths and local commits as intentional. Do not translate a general repair/update
   request into `exec`, and never invent `reset --hard`, destructive checkout, clean, force push, or
   credential inspection.
4. Use the closed operation matching the current request: `status`, `fetch`, `update`, `merge`,
   `test`, `build`, `commit`, `push`, or `pr`. The compiled dispatcher—not this file—validates every
   root, ref, remote, path, command ID, timeout, and receipt.
5. For an affirmative request to troubleshoot, debug, modify, or fix this Tentacle's own code:
   - Inspect files and logs using bounded workspace tools (`list_files`, `read_file`, `search_files`, `qmd_search`).
   - Make contained edits using `edit_file` (or `/write`, `/edit`).
   - Run tests and validation using typed `{"operation":"test"}` or `{"operation":"build"}`.
   - For version control, use `{"operation":"commit", "message": "...", "paths": [...]}` and `{"operation":"push"}`.
6. A source update or code modification never changes the process already answering the operator. If
   source/build validation succeeds, report the results and explain that the current process is still
   running the previous binary. To redeploy:
   - For native / local setups: stop the `uwubot` process cleanly and relaunch `./uwu.sh`.
   - For containerized setups: rebuild the container image (`docker build` or container compose) and restart the container.
7. If status reports no contained Git checkout or trusted Git, do not claim self-sync is available.
   The stock container has no Git checkout and ships neither `git` nor `gh`; rebuild and redeploy its
   image instead. A source bind mount does not update the running binary by itself.

If a bounded validation phase runs out of time, report completed and pending step IDs. Continue with
a later typed `test` or `build`; do not call the partial run successful.

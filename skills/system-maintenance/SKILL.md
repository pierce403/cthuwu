---
name: system-maintenance
description: Diagnose this Tentacle's source checkout, installed tools, version, and safe repair path.
---

# System maintenance

Use this skill only for an authenticated operator request to diagnose, update, validate, or repair
this Tentacle's own Cthuwu checkout.

1. Read this file before calling `repository_maintenance`.
2. Start with `{"operation":"status"}` unless the runtime has already routed an exact common update
   phrase through the deterministic maintenance workflow. Report the canonical repository root,
   HEAD, branch, tracked ref, dirty entries, canonical/fork topology, ahead/behind counts, sanitized
   remotes, and Git/`gh` availability from the receipt.
3. Treat dirty paths and local commits as intentional. Do not translate a general repair/update
   request into `exec`, and never invent `reset --hard`, destructive checkout, clean, force push, or
   credential inspection.
4. Use the closed operation matching the current request: `status`, `fetch`, `update`, `merge`,
   `test`, `build`, `commit`, `push`, or `pr`. The compiled dispatcher—not this file—validates every
   root, ref, remote, path, command ID, timeout, and receipt.
5. For an affirmative request to fix/repair this Tentacle's own source, inspect only relevant
   files/logs through bounded workspace tools, make one contained exact-text edit per authorized
   turn, then use typed `test`/`build`. A Git “update yourself” request authorizes only the typed Git
   update, not an edit. Never claim a command ran without its receipt.
6. A source update never changes the process already answering the operator. If source/build
   validation succeeds, report both commit IDs and say the current process is still old. Cthuwu has
   no generic service restart hook: the safe documented action is to stop it cleanly and relaunch
   `./uwu.sh`.
7. If status reports no contained Git checkout or trusted Git, do not claim self-sync is available.
   The stock container has no Git checkout and ships neither `git` nor `gh`; rebuild and redeploy its
   image instead. A source bind mount does not update the running binary by itself.

If a bounded validation phase runs out of time, report completed and pending step IDs. Continue with
a later typed `test` or `build`; do not call the partial run successful.

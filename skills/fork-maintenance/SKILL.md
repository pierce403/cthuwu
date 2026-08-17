---
name: fork-maintenance
description: Merge canonical Cthuwu upstream into a long-lived operator fork without clobbering fork work.
---

# Fork maintenance

Read `skills/system-maintenance/SKILL.md` and `skills/git-maintenance/SKILL.md` first.

1. Call typed `status`. Confirm `origin` identifies the operator fork and a separate remote identifies
   the manifest-pinned canonical `pierce403/cthuwu`.
2. If the canonical remote is absent, typed `fetch` or `update` may add the manifest-pinned URL under
   an unused `upstream`-style name. It never rewrites an existing remote.
3. Refuse an automatic update when the worktree is dirty. Preserve every path and tell the operator
   what blocks safe integration.
4. `update` fetches fork and upstream, inspects merge-base/ahead/behind, and normally merges the
   corresponding upstream branch. It does not rebase or rewrite the fork.
5. On conflict, report the exact bounded conflict list. Inspect each file's intent, preserve both
   fork-specific and upstream behavior, edit deliberately, stage explicit paths, and validate.
   Never resolve by wholesale replacing one side.
6. A successful fork update pushes the current fork branch only after prescribed validation. No
   force push is available.

Report old/new commit, merge base, divergence, validation steps, whether the fork push succeeded,
and that the running binary still needs a clean stop and `./uwu.sh` relaunch.

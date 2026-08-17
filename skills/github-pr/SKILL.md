---
name: github-pr
description: Prepare a scoped fork branch and submit a real upstream Cthuwu PR through authenticated gh.
---

# GitHub pull requests

Read `skills/system-maintenance/SKILL.md`, `skills/git-maintenance/SKILL.md`, and
`skills/repository-validation/SKILL.md` first.

Use typed `pr` only when the authenticated operator explicitly asks to contribute the current fix
upstream. Supply a valid topic `branch`, concise `title`, non-empty `body`, one-line
`commit_message`, explicit scoped `paths`, and optional `base`.

The dispatcher creates or reuses only the requested current topic branch, stages only those paths,
commits, runs manifest-pinned validation, checks bounded `gh --version` and
`gh auth status --hostname github.com`, pushes the topic branch without force, then runs
`gh pr create` against manifest-pinned `pierce403/cthuwu`. If `gh` is absent or unauthenticated, the
prepared local branch/commit remains intact and unpushed. If validation fails, nothing is pushed.

Claim a PR only when the receipt contains a successful command and a verified canonical
`https://github.com/pierce403/cthuwu/pull/<number>` URL. Unit-test mock output is never a live PR.
Never print tokens, credential files, SSH material, or raw authentication output.

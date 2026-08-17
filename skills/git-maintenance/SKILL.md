---
name: git-maintenance
description: Inspect, fetch, update, merge, commit, and push Cthuwu through the typed safe-Git dispatcher.
---

# Safe Git maintenance

Read `skills/system-maintenance/SKILL.md` first. Use only `repository_maintenance`; a natural Git
request is not permission to synthesize an `exec` shell command.

- `status` is read-only and reports current HEAD/branch, dirt, remotes, topology, and divergence.
- `fetch` fetches configured safe remotes and prunes remote-tracking refs without changing HEAD.
- `update` verifies a clean checkout, fetches canonical metadata, fast-forwards a non-diverged
  canonical checkout, or normally merges canonical upstream into a long-lived fork. Canonical
  divergence fails closed. Fork conflicts remain in progress for deliberate resolution.
- `merge` requires the verified canonical-upstream remote and an existing branch. It never chooses
  a conflict side.
- `commit` requires a one-line message and explicit repository-relative paths. It verifies both the
  existing dirty set and final staged set, so it never stages `.`, a symlink, traversal, `.git`, a
  root-wide scope, or unrelated/pre-staged paths.
- `push` requires verified noncanonical fork `origin`, matching fetch/push repository identities,
  and the checked-out branch. It never forces.

The dispatcher rejects external Git directories, symlinked roots, suspicious executable/path Git
configuration, credential-bearing or unsupported network remotes, malformed refs, dirty automatic
updates, unsafe remote fetch refspecs, uncontained submodules, unbounded output, and timeouts.
Receipts sanitize remote URLs and known secret forms. Do not ask for tokens, credential files,
private keys, or credential-helper output.

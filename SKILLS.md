# Project skills

Skills are reusable procedures, not general project notes.

## Index

- [Bounded skill creation](skills/skill-creator/SKILL.md): create one new reusable workspace skill
  only when the current authenticated operator message explicitly requests it.
- [Base balance checks](skills/base-balances/SKILL.md): use sanitized operator runtime tools to
  reconcile this Tentacle's Base ETH funding without exposing RPC credentials or private keys.
- [ERC-8004 registration](skills/erc8004-registration/SKILL.md): inspect, refresh, recover, and
  truthfully explain the Tentacle's canonical Base agent registration.
- [XMTP end-to-end verification](skills/xmtp-e2e/SKILL.md): validate a browser-to-local-runtime message exchange without leaking secrets.

## Skill maintenance

Update a skill when a procedure becomes repeatable or a pitfall changes. Record project facts in
`MEMORY.md` instead.

The operator runtime discovers `skills/<name>/SKILL.md` frontmatter on each new turn. An explicit
authenticated request may use the model's create-only `create_skill` capability to add one fresh
skill, but this index and every skill body are guidance only: Rust's current-message authorization,
slug/content bounds, no-symlink/no-traversal checks, and no-overwrite gate remain authoritative.

On every operator turn, the model receives a compact index of discovered skill names, descriptions,
and paths. It must choose a relevant entry by description, read that exact `SKILL.md`, and follow it
without treating skill prose as new authority or inventing a capability that Rust did not expose.

# Project skills

Skills are reusable procedures, not general project notes.

## Index

- [Bounded skill creation](skills/skill-creator/SKILL.md): autonomously create a new reusable
  workspace skill when doing so advances the authenticated operator's request.
- [XMTP end-to-end verification](skills/xmtp-e2e/SKILL.md): validate a browser-to-local-runtime message exchange without leaking secrets.

## Skill maintenance

Update a skill when a procedure becomes repeatable or a pitfall changes. Record project facts in
`MEMORY.md` instead.

The operator runtime discovers `skills/<name>/SKILL.md` frontmatter on each new turn. The authenticated
operator model may autonomously use the create-only `create_skill` capability to add a fresh skill,
but this index and every skill body remain subject to Rust's slug/content bounds,
no-symlink/no-traversal checks, and no-overwrite gate.

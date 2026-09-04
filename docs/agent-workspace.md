# Markdown agent and coaching workspace

The implementation keeps the existing Rust loop. Bash, Markdown skills, and a small Python CLI
provide extensibility; pi and Hermes are design inspirations, not additional running harnesses.
The primary mission is helping willing acolytes pursue goals they choose.

Upgrade the checkout, then restart the Tentacle through `./uwu.sh` using its existing data directory.
A GitHub push or Pages deployment does not restart a running node. Existing protected SOUL files
are preserved; the new coaching instructions also enter the runtime prompt. Python 3 with SQLite
FTS5 and Git are required for the workspace CLI and are included in the container.

## Configure the workspace

Set `UWUBOT_OPERATOR_ROOT` to the operator-controlled workspace, separate from `UWUBOT_DATA_DIR`.
First startup seeds missing files and directories without replacing your edits:

| Location | Purpose |
|---|---|
| `MISSION.md` | Operator coaching mission, tone, practices, and priorities; also supplied to public coaching |
| `ENVIRONMENT.md` | Verified commands, services, prerequisites, observation dates, and failures |
| `HEARTBEAT.md` and `tasks/` | Monitoring instructions, relevance criteria, checkpoints, and upstream cursors |
| `knowledge/` | Sourced research with dates and reported/observed/inferred distinctions |
| `skills/<name>/SKILL.md` | Reusable procedures; compact index in context, bodies read when relevant |
| `scripts/workspace.py` | Discoverable CLI for indexing, retrieval, skill revisions, and upstream checks |
| `state/agent/` under the **data directory** | Protected SOUL, per-operator profiles, session receipts, and authorized task registrations |
| `state/coaching/<inbox>/` under the **data directory** | Private acolyte goals and reminder preferences |

`AGENTS.md` or the existing supported workspace instruction file supplies local conventions.
Markdown is guidance and source material. It cannot authorize shell execution, transfers, recurring
work, credential changes, or access to another person's data.

An ordinary operator request can discover installed commands through `--help`, run bounded Bash,
inspect results, and record useful findings. The node account's OS permissions remain the shell
boundary. Public acolytes receive no shell or workspace-file tool.

## Retrieve knowledge and learn skills

From the workspace:

```bash
python3 scripts/workspace.py --help
python3 scripts/workspace.py index
python3 scripts/workspace.py search "habit planning"
python3 scripts/workspace.py skill morning-routine --description "Plan a small morning routine" --file tasks/verified-procedure.md
```

The first two retrieval commands use keyword search. To enable semantic retrieval, install an
embedding model in the local Ollama service, then use the **same model ID** for indexing and queries:

```bash
python3 scripts/workspace.py index --model <installed-embedding-model>
python3 scripts/workspace.py search "building a sustainable routine" --model <installed-embedding-model>
```

Embeddings use the loopback [Ollama embedding API](https://docs.ollama.com/api/embed), with proxies
disabled. SQLite stores vectors, FTS5 text, content hashes, and source locations in
`.knowledge-index/index.sqlite`. Hybrid search combines keyword and cosine rankings and returns
paths, line numbers, and excerpts. This is a small-workspace implementation using an exact vector
scan, not a hosted vector service. The database is disposable: rerun `index` to rebuild it.

Only Markdown below `knowledge/` and active `skills/` is indexed. Files marked `private: true`,
retired skills, revisions, hidden paths, and symlinks are excluded. Retrieval rechecks source hashes
and existence, so edits and deletion take effect before the next index run. Credentials, sessions,
and private acolyte notes are outside this corpus. Do not place secrets in shared knowledge files.
Private acolyte context is currently an exact inbox-scoped Markdown lookup, not semantic retrieval.

The operator prompt encourages learning a reusable skill after verified work. Reusing `skill` with
the same name archives the old file under `.revisions/<content-hash>.md`; `--retire` removes a skill
from discovery. Restore a revision by reviewing and copying the desired instructions back into the
current file, then reindex. Include prerequisites, verification steps, sources, and known pitfalls;
remove personal details before sharing a procedure. The older typed `create_skill` helper remains
create-only; the CLI provides the revision lifecycle through the existing authorized Bash path.

## Run and steer background work

Send these in the operator DM:

```text
/task run Inspect the environment and write verified capabilities to ENVIRONMENT.md
/task add 86400 Read HEARTBEAT.md and check the configured upstreams; notify me only about relevant changes
/task list
/task pause <id>
/task steer <id> <updated request>
/task resume <id>
/task remove <id>
```

Tasks have a separate 15-minute budget, a 24-tool-call ceiling, and one execution slot per Tentacle.
Recurring intervals range from one minute to one year; at most 100 registrations are retained.
Task controls remain available while a job runs. Ordinary foreground requests receive a busy
response instead of holding up a following pause/steer command. One-off starts and results return to the authorizing
operator inbox. Recurring jobs can return exactly `[NO_UPDATE]` to suppress an unchanged result.

Registrations are atomically persisted and bound to the operator's authorization generation.
Restart pauses interrupted jobs, and failures or uncertain timeouts pause recurring work. Resume
is explicit. The next run sees bounded prior session receipts for the same inbox and model route;
inspect unknown effects before repeating an action. Sessions retain at most six exchanges/32 KiB,
so write durable task plans and next actions in Markdown. Changing the selected model/provider
clears prior session context to prevent unintended egress to a new route.

For each upstream, keep a trusted checkout below the workspace and run:

```bash
python3 scripts/workspace.py upstream <repository-relative-path>
```

The helper accepts a credential-free HTTPS GitHub `origin`, fetches its default-branch head,
records up to 30 commit summaries with source/date/head, and persists the inspected commit.
Unchanged heads return `changed: false`. The agent interprets relevance using `HEARTBEAT.md` and
can inspect diffs with Bash. Fetching never checks out, installs, deploys, or restarts upstream code.
Registration through `/task` is required; editing heartbeat Markdown alone does not schedule work.

## Coach acolytes with explicit memory

Acolytes can use ordinary phrases:

```text
remember my goal: walk for ten minutes after lunch
show my goal
update my goal: walk after lunch three times this week
check in daily
check in weekly
pause check-ins
forget my goal
```

Goals are explicitly reported notes for the exact authenticated inbox. The selected model receives
only that acolyte's goal plus the operator's mission during conversation. The node operator can
access local notes; the save receipt explains this. Goal updates retain any existing check-in
preference. Check-ins require affirmative daily/weekly opt-in and start after that interval.
Pause and deletion work without a model or funding connection. Forgetting the goal also stops
check-ins; the broader confirmed local-data deletion flow removes it too. Existing chat copies
are subject to transport retention rather than being erased by local deletion.

Check-ins use an at-most-once local scheduling claim. A crash or ambiguous delivery can skip a
reminder rather than send it twice. They ask about progress without exposing goal text in a
notification. Operators should support acolyte-chosen next actions, acknowledge uncertainty, and
keep recruitment and Branding separate from progress on the person's goal.

## Operator panel, referrals, and transfer

Open `/operator/` with the existing browser identity. One SDK client watches all DMs, including
inactive and new conversations. Save names, switch Tentacles, and keep each conversation's draft.
Stream/catch-up overlap is deduplicated; disconnects retry with backoff and resume performs catch-up.
Drafts remain in memory, so credentials typed into a draft are not saved in browser preferences.
The panel currently catches up the latest 200 DMs, 80 messages per page, and keeps up to 1,000
messages per conversation within the 14-day disappearing window. A saved contact is not proof of
operator authority; the Tentacle's runtime checks every privileged request.

The selected Tentacle has a visible `#t=<tentacle-wallet>&r=<your-wallet>` referral link. Copy/Share
first requires a fresh authenticated referral-activation acknowledgement from that Tentacle.
The recipient sees the referrer and intended route before connecting. Existing active Branding
takes precedence, and the existing canonical identity, attribution, and transaction-consent checks
remain in the onboarding path. An unavailable target leaves a visible retry state.

Configure proactive sharing reminders with `/referrals daily`, `weekly`, `off`, `snooze 7`,
`quiet 22 8`, or `quiet off`. Quiet hours are UTC. Reminders use the current registered operator's
link; former-operator reminders cannot be delivered to the new operator. Eligible acolytes can
receive an earlier Branding offer, while durable declines and explicit transaction consent remain
required.

Transfer with `/operator-switch <address-or-ENS>`, review the resolved wallet/inbox, then send the
returned `/operator-switch confirm <token>` within five minutes. Resolution is checked again before
the atomic replacement. The former operator receives the result and the new operator gets a
best-effort welcome DM. All installations of the destination inbox gain authority. Old jobs and
stale messages remain fenced; profiles/history are not copied. Offline recovery is
`./uwu.sh --data-dir <existing-data-directory> operator switch <address-or-ENS>` while stopped.
Restart without a conflicting old `--operator`/`UWUBOT_OPERATOR` value.

## Configure environment values and backup keys

```text
/env list
/env get VENICE_API_KEY
/env set UWUBOT_PROVIDER ollama
/env set UWUBOT_MODEL <model-id>
/env set VENICE_API_KEY <dedicated-key>
/env add VENICE_API_KEY backup <another-dedicated-key>
/env add UWUBOT_MODEL_API_KEY backup <compatible-endpoint-key>
/env disable VENICE_API_KEY backup
/env enable VENICE_API_KEY backup
/env remove VENICE_API_KEY backup
/env unset VENICE_API_KEY
/env set CTHUWU_RPC_ENDPOINT <Base-mainnet-HTTPS-endpoint>
/env unset CTHUWU_RPC_ENDPOINT
/env set TOOL_SERVICE_TOKEN <dedicated-tool-token>
```

Provider/model changes and the single Base RPC endpoint use their existing hot-apply adapters.
Unsetting RPC restores the public Base endpoint. Named backup slots apply to Venice and the
already-configured OpenAI-compatible endpoint, not arbitrary provider changes. `set` replaces the
primary slot; `add` appends without replacement, up to eight slots. Legacy key commands/stores
remain compatible. Venice aliases normalize the variable name without changing the value.

Venice candidates must authenticate and pass the TEE check before acceptance. Other compatible
model keys are checked on first use. At most three compatible keys are tried per request; their
budgets preserve local fallback time. HTTP 401/403 disables the rejected slot, 429 cools it for
five minutes, and other failures cool it for one minute. Re-enable explicitly after correcting
rejected credentials. Retries after ambiguous timeouts can still incur another billable request.
Failover never switches to an unrelated remote provider; Venice privacy checks still apply.

Values persist outside the workspace and all reads are redacted. Only `TOOL_*` values enter Bash
children, as environment data, with exact-value output redaction; loader/runtime variables are
closed. Keys sent through XMTP can remain in transport history: use dedicated, revocable keys.
An authenticated acolyte can donate a bounded validated backup with
`/env donate VENICE_API_KEY <key>`; this does not replace a slot, select a provider, earn an
unverified reward, or grant configuration authority.

## Verification still required on a running installation

The local suites cover persistence, scoped retrieval, key failover, transfer fencing, scheduling,
panel routing, and referral controls. They do not prove production XMTP delivery, mobile wallet
onboarding through a funded Branding mint, or retrieval quality with a real embedding model.
Those release gates remain open in `FEATURES.md`. The current implementation also keeps public
personal-note retrieval separate from the operator's semantic corpus and does not install pi,
Hermes, remote tool adapters, or an embedding model automatically.

# Markdown agent and coaching workspace

The implementation keeps the existing Rust loop. Bash, Markdown skills, and a small Python CLI
provide extensibility; pi and Hermes are design inspirations, not additional running harnesses.
The primary mission is helping willing acolytes pursue goals they choose.

Start or restart the Tentacle through `./uwu.sh` using its existing data directory. A GitHub push or
Pages deployment does not restart a running node. Existing protected SOUL files are preserved;
coaching and workspace conventions also enter the runtime prompt. Python 3 with SQLite FTS5 and
Git are required for the workspace helpers and are included in the container.

## Configure the workspace

Set `UWUBOT_OPERATOR_ROOT` to the operator-controlled workspace, separate from `UWUBOT_DATA_DIR`.
First startup seeds missing files and directories without replacing your edits:

On later starts, shipped `scripts/code.py` and `scripts/workspace.py` upgrade when their contents
still match the recorded version. Protected hash receipts identify that version. Locally edited or
unrecognized helpers stay intact and appear as explicit helper divergence in the operator context;
review them before adopting an updated helper. `CODE.md` and other operator Markdown remain editable.

| Location | Purpose |
|---|---|
| `MISSION.md` | Operator coaching mission, tone, practices, and priorities; also supplied to public coaching |
| `ENVIRONMENT.md` | Verified commands, services, prerequisites, observation dates, and failures |
| `CODE.md` | Configured prime tentacle upstream, source state, and reasons for local divergence |
| `WORKSPACE_LOG.md` | Reasons for local Git checkpoints after changed tool calls |
| `HEARTBEAT.md` and `tasks/` | Monitoring instructions, relevance criteria, checkpoints, and upstream cursors |
| `knowledge/` | Sourced research with dates and reported/observed/inferred distinctions |
| `skills/<name>/SKILL.md` | Reusable procedures; compact index in context, bodies read when relevant |
| `scripts/workspace.py` | Discoverable CLI for indexing, retrieval, skill revisions, and upstream checks |
| `scripts/code.py` | Source checkout, reviewed integration, and workspace release installation |
| `code/` | Independent Git checkout on this Tentacle's local branch |
| `tmp/` | Scratch files and subprocess temporary files |
| `tools/` | Workspace home, package installations, caches, and tool configuration |
| `releases/` | Installed source-specific runtime bundles and the next-start selection |
| `state/agent/` under the **data directory** | Protected SOUL, per-operator profiles, session receipts, and authorized task registrations |
| `state/coaching/<inbox>/` under the **data directory** | Private acolyte goals and reminder preferences |

`AGENTS.md` or the existing supported workspace instruction file supplies local conventions.
Markdown is guidance and source material. `CODE.md` supplies the validated upstream configuration;
it cannot grant shell authority, transfer operators, install itself, or access another person's
data. The daily review is a runtime default controlled through `/task`, not arbitrary executable
instructions discovered in a heartbeat file.

An ordinary operator request can discover installed commands through `--help`, run bounded Bash,
inspect results, and record useful findings. The node account's OS permissions remain the shell
boundary. Public acolytes receive no shell or workspace-file tool.

## Keep work and tools inside the workspace

The agent's default working directory, temporary directory, home, package prefixes, and caches are
inside `UWUBOT_OPERATOR_ROOT`. Use `tmp/` instead of `/tmp`, and install tools into `tools/` instead
of the system or the host user's home. Workspace paths include `tools/home`, `tools/venv`,
`tools/pip`, `tools/npm`, `tools/npm-cache`, `tools/pnpm`, `tools/pnpm-store`, `tools/cargo`,
`tools/rustup`, `tools/brew`, and `tools/xdg`. Child processes receive matching environment values
and a PATH that discovers workspace tools. Package directories are prepared; their existence does
not mean Brew, a Python virtual environment, Rust, or an embedding model is installed.

Environment discovery should record verified capabilities in `ENVIRONMENT.md`. Use a workspace
virtual environment or package prefix for missing tools. Changing the surrounding OS requires an
explicit operator request to modify that environment; ordinary task work must stay in the workspace.
These defaults guide package managers and the agent, while OS permissions still bound Bash. An
absolute path or a tool that ignores its environment can leave the root, so this is not a filesystem
sandbox. Protected wallet, XMTP, credentials, sessions, and personal notes remain in the separate
existing data directory; they are not copied into the workspace or its Git history.

The workspace has its own Git history, separate from the nested `code/` repository. Mutating tool
calls produce a workspace checkpoint and an entry in `WORKSPACE_LOG.md`. A shell invocation can change
several files; the checkpoint records the resulting change together rather than claiming a commit
for every intermediate filesystem write. Temporary files, installed tools, build/release output,
the source checkout, derived indexes, and secret material are excluded. Git records local history;
it does not publish the workspace or source branch automatically. Inspect the checkpoint receipt
if a command or commit fails before repeating work.

Existing Git history and staged work are preserved. The journal records changes observed after its
startup baseline; it does not adopt pre-existing dirty files as new agent work. Linked/external Git
directories and symlinked metadata are rejected. If a tool touches an existing dirty/staged path,
the journal leaves it uncommitted and reports that operator review is needed. Reasons describe the
runtime action without copying the operator's prompt, command, credentials, or private conversation
into commit messages.
Ignored directories and obvious secret filenames are excluded, but Git is not a secret classifier:
do not put sensitive material in ordinary workspace notes. Checkpoints are bounded to 4,096 files
and 4 MiB per file; rotate the reason log when requested.

## Review and install source updates

The Tentacle keeps its source under workspace `code/` on its own branch. `CODE.md` defaults to
`https://github.com/pierce403/cthuwu`; the configured upstream is called the **prime tentacle**.
An operator can edit the validated GitHub upstream setting to follow another compatible source:

```markdown
---
upstream: https://github.com/pierce403/cthuwu.git
branch: main
---
```

Keep personal rationale in the Markdown body. The helper refreshes its generated source-state
section between `<!-- code-state:start -->` and `<!-- code-state:end -->`, preserving surrounding
prose. The local branch is named `tentacle`; changing the upstream to an unrelated repository is
rejected instead of replacing local history.

Existing Markdown and intentional local code are preserved. An unavailable upstream or missing
build tool produces a concrete failure receipt rather than a claim that the update succeeded.

Send `/update` in the authenticated operator DM, or `/update <requested functionality or commit>`
to override a preference. Common requests such as “update yourself” use the same managed path.
It registers a background job, so `/task list`,
`pause`, and `steer` remain available during a long fetch or build. A working model is needed for
the agent's review and explanation. The task inspects the fetched source and local branch, then:

- If there is no local divergence, fast-forward to the prime tentacle and build/install the result.
- If the branch differs, review candidate changes, adopt useful functionality while preserving
  intentional local work, and record deferred changes and the reasons in `CODE.md`.
- If the operator disagrees, accept their requested functionality and update the rationale. The
  Tentacle may complain in character; it must still follow the instruction and report actual results.

The final reply briefly names the source in use, accepted and deferred changes, reasons for local
differences, and the install/restart state. Pride in improvements must refer to verified changes;
speculative ideas and unmeasured benefits remain labeled as such. Dirty work is preserved; a conflict
stops integration with a failure receipt rather than resetting or overwriting the durable branch.

Installation builds a coherent Rust binary and Node transport bundle under `releases/<commit>/`.
`releases/active.json` selects the installed commit for the next launch. It does not replace the
running process in place. Stop the current Tentacle cleanly, then relaunch with the same workspace
and data directory; `uwu.sh` and the container entrypoint validate the paired release paths and
hashes before selecting it. Until that restart succeeds, distinguish the running binary from the
checked-out and installed commits. Use the validated Rust 1.98 toolchain and Node 22 or newer with
the project's locked dependencies. The stock runtime image includes Git/Python but not the Rust compiler; provide a
workspace-local build toolchain or rebuild/redeploy the image instead of claiming a successful build.

The agent uses the small CLI directly through Bash; these commands are also available for deliberate
local maintenance from the workspace:

```bash
python3 scripts/code.py --help
python3 scripts/code.py status
python3 scripts/code.py review
python3 scripts/code.py update
python3 scripts/code.py accept <full-commit-sha> --reason "Verified improvement requested by the operator"
python3 scripts/code.py defer <full-commit-sha> --reason "Specific compatibility tradeoff"
python3 scripts/code.py install
```

`init` creates the checkout when needed. `review` fetches bounded commit/diff context without
adoption or installation. `accept` and `defer` select up to 20 full commit IDs from the current
review and require a reason of at most 800 characters. Accepted selections are prepared in a
temporary worktree below workspace `tmp/`, tested, and built before advancing the durable branch.
Merge commits need deliberate review of their constituent changes; the helper will not guess a
cherry-pick parent. Missing dependencies or conflicts preserve the prior installed release.

## Retrieve knowledge and learn skills

From the workspace:

```bash
python3 scripts/workspace.py --help
python3 scripts/workspace.py index
python3 scripts/workspace.py search "habit planning"
python3 scripts/workspace.py skill morning-routine --description "Plan a small morning routine" --file tasks/verified-procedure.md
```

The first two retrieval commands use keyword search. To enable semantic retrieval, run a local
Ollama service with model storage in workspace `tools/ollama/models` (`OLLAMA_MODELS` in the
workspace environment), install an embedding model there, then use the **same model ID** for
indexing and queries. A separately managed host Ollama service uses its own storage settings;
configure those deliberately before downloading models through it.

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
/task interval <id> 172800
/task steer <id> <updated request>
/task resume <id>
/task remove <id>
```

Tasks have a separate 15-minute budget, a 24-tool-call ceiling, and one execution slot per Tentacle.
Ordinary shell calls default to 120 seconds; the configurable per-tool ceiling defaults to 900.
The source helper bounds one build/validation attempt to ten minutes. All calls still fit within
the job's remaining budget; a longer timeout cannot extend a foreground XMTP reply deadline.
Recurring intervals range from one minute to one year; at most 100 registrations are retained.
Task controls remain available while a job runs. Ordinary foreground requests receive a busy
response instead of holding up a following pause/steer command. One-off starts and results return to the authorizing
operator inbox. Recurring jobs can return exactly `[NO_UPDATE]` to suppress an unchanged result.

Each active operator authorization gets a default prime-tentacle review, first due one day after
registration and then every 86,400 seconds. It runs `python3 scripts/code.py review`, considers
useful upstream changes and local coaching improvements, and records sourced findings in
`knowledge/prime-review.md`. Daily review can inspect and write notes; it does not adopt or install
source. `/update` supplies that explicit operator request. Use `/task list` to find the review ID,
`/task interval <id> <seconds>` to change cadence, or `pause`/`remove` to disable it. Paused and
removed defaults stay disabled after restart; a removed builtin remains visible as a tombstone and
can be restored with `resume`. A new operator authorization gets its own review, never the former
operator's task or private history. The existing 100-registration store limit still applies.

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
Additional registrations use `/task`; editing heartbeat Markdown alone does not schedule work.

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

Transfer with `/operator <address-or-ENS>`, review the resolved wallet/inbox, then send the returned
`/operator confirm <token>` within five minutes. The address must have a real registered inbox on
the configured XMTP network. Confirmation resolves the original ENS name or address again and
rejects a missing or changed wallet/inbox binding before replacing authority. `/operator-switch`
remains a compatibility alias. The former operator receives the result and the new operator gets a
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

Venice candidates must authenticate and pass the configured privacy checks before acceptance; TEE is the default. Other compatible
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

## Diagnose a tentacle without a working model

Use `/doctor` (or `/doctor fix`) for fixed diagnostics and safe repairs; `/doctor check`
reports without repairing. The exact question “is your venice cred working?” also runs
check mode directly. These commands require the authenticated operator lane and do not
rely on an LLM choosing tools.

A configured credential only means a key is stored. Doctor probes the selected provider and
local Ollama separately, using a tiny synthetic completion with a tool schema, never private
conversation history. Venice probes use the current credential pool and fresh catalog checks, plus attestation when TEE mode is selected. Failures distinguish authentication/access, credit, rate limits, connectivity, model
compatibility, and attestation problems without printing keys or raw provider errors. Up to
three enabled credential slots are tested, including cooling-down slots; disabled slots stay
disabled. Each probe has a 90-second cap within a 420-second inference budget and the incoming
request deadline. Probes can incur small provider charges. A successful fallback is never
reported as proof that Venice works.

Safe repair clears cooldowns only for successfully tested, still-current credentials/routes
and creates missing workspace-local temporary/cache/tool directories. It does not replace keys,
switch models, weaken privacy checks, install packages, or restart the node. Doctor checks
workspace/source helper presence and tool availability; versions/build integrity still require
`/update`. Base RPC and ERC-8004 reports are configuration/cached status, not live chain proofs.
XMTP round-trip delivery still needs a live exercise. A rejected key, depleted account, absent
model, missing executable, or failed attestation remains an actionable finding, not a claimed fix.

After deploying this code, restart through `./uwu.sh` using the existing data directory, then run
`/doctor`. No running node is upgraded merely by pushing GitHub main.

## Explicit Venice privacy selection

`/env set UWUBOT_VENICE_PRIVACY standard` disables TEE attestation for both public and operator
inference. TLS, authenticated exact-model catalog validation, and function-calling capability checks
remain. This is an explicit operator choice, never an automatic fallback. It persists across restart;
legacy configurations still default to `tee`. Model and credential slots are preserved. Switching
privacy creates a separate operator session route so prior dialogue is not silently replayed.

To use Venice GLM 5.3 Flash, send each command as a separate operator XMTP message:

```text
/env set UWUBOT_VENICE_PRIVACY standard
/env set UWUBOT_PROVIDER venice
/env set UWUBOT_MODEL z-ai-glm-5-3-flash
/doctor check
```

The [Venice catalog](https://docs.venice.ai/models/text) lists that exact model ID. Standard mode
makes no TEE or E2EE claim. Restore attestation with `/env set UWUBOT_VENICE_PRIVACY tee`, then
select a TEE-capable model. `/env list` reports the active privacy policy.

## Recovery when inference is unavailable

`/force-update` queues a fixed one-shot job without any model call. It uses the compiled-in Python
helper, fetches the **main** branch of the prime URL in `CODE.md` (ignoring its ordinary review branch),
and builds/tests a paired Rust binary and Node sidecar under workspace-local directories. Only a
validated release becomes active for the next start. Local source fast-forwards when clean and
compatible; otherwise dirty files and divergent commits remain intact in `code/`, excluded from
the installed upstream release. It never resets local work or publishes a fork.

The task sends a starting notice and a result to the authorizing operator. Use `/task list` or
`/task pause <id>` during execution. Failed/interrupted runs pause; restart never silently repeats
them. They cannot be steered into arbitrary commands or made recurring. Builds still need Git,
Python, Rust, Node, npm, network access, disk space, and the existing bounded build budget; no system
packages are installed. Failed builds preserve the previous active release. `CODE.md` records the
installed commit, source divergence, and why main was selected; workspace changes are checkpointed.

Installation does not restart the live process. Restart through `./uwu.sh` (or the container's
entrypoint) with the same protected data and workspace to activate the release. **Older binaries
cannot recognize this new command:** update and restart the host checkout once to bootstrap it.

## Timeouts and concise replies

Default XMTP replies now have a 600-second envelope (`UWUBOT_REPLY_TIMEOUT_MS`, allowed 2–900
seconds). Public work remains capped at 240 seconds, with up to 120 seconds per remote candidate.
Venice operator candidates default to 300 seconds (`UWUBOT_VENICE_TIMEOUT_SECONDS`); generic
OpenAI-compatible candidates default to 180 seconds. Ollama defaults to 90 seconds per phase.
Catalog and attestation phases allow 30 seconds each, always within the candidate's total budget.
The operator route reserves 211 seconds for local fallback, a tool phase, and final completion.
Existing explicit environment overrides remain effective; increasing defaults does not override a
shorter configured bridge or provider timeout. Updated Rust and sidecar builds must run together.

Routine public replies target 1–3 short sentences, normally under 80 words; operator replies target
under 120 words. Both answer directly and avoid repeated introductions and persona boilerplate.
Requested detail, complete code, tool arguments, and essential diagnostic receipts remain available;
these are prose instructions, not destructive truncation of structured output. A slow or unavailable
provider can still time out; longer budgets do not establish service health.

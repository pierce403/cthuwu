# syntax=docker/dockerfile:1

FROM node:22-bookworm-slim AS agent-build
WORKDIR /build/agent
COPY agent/package.json agent/package-lock.json ./
RUN npm ci
COPY agent/tsconfig.json agent/tsconfig.build.json ./
COPY agent/src ./src
RUN npm run build && npm prune --omit=dev

FROM rust:1.98-bookworm AS rust-build
WORKDIR /build/cthuwu
COPY repository-maintenance.json /build/repository-maintenance.json
COPY cthuwu/Cargo.toml cthuwu/Cargo.lock ./
COPY cthuwu/crates ./crates
COPY cthuwu/agent-files ./agent-files
COPY scripts/workspace.py /build/scripts/workspace.py
COPY scripts/code.py /build/scripts/code.py
COPY cthuwu/src ./src
RUN cargo build --package cthuwu --locked --release

FROM node:22-bookworm-slim AS runtime
WORKDIR /data
RUN apt-get update \
    && apt-get install -y --no-install-recommends ripgrep python3 git ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && mkdir -p /opt/cthuwu/agent /data /workspace \
    && chown -R node:node /opt/cthuwu /data /workspace
COPY --from=rust-build /build/cthuwu/target/release/uwubot /usr/local/bin/uwubot
COPY scripts/release-entrypoint.py /opt/cthuwu/release-entrypoint.py
COPY --from=agent-build --chown=node:node /build/agent/dist /opt/cthuwu/agent/dist
COPY --from=agent-build --chown=node:node /build/agent/node_modules /opt/cthuwu/agent/node_modules
COPY --from=agent-build --chown=node:node /build/agent/package.json /opt/cthuwu/agent/package.json
COPY --chown=node:node AGENTS.md MEMORY.md README.md SKILLS.md /workspace/
COPY --chown=node:node skills /workspace/skills

ENV UWUBOT_DATA_DIR=/data \
    UWUBOT_OPERATOR_ROOT=/workspace \
    UWUBOT_SIDECAR=/opt/cthuwu/agent/dist/index.js \
    UWUBOT_XMTP_ENV=production

USER node
VOLUME ["/data", "/workspace"]
ENTRYPOINT ["python3", "/opt/cthuwu/release-entrypoint.py"]

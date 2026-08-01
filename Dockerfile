# syntax=docker/dockerfile:1

FROM node:22-bookworm-slim AS agent-build
WORKDIR /build/agent
COPY agent/package.json agent/package-lock.json ./
RUN npm ci
COPY agent/tsconfig.json agent/tsconfig.build.json ./
COPY agent/src ./src
RUN npm run build && npm prune --omit=dev

FROM rust:1.97-bookworm AS rust-build
WORKDIR /build/cthuwu
COPY cthuwu/Cargo.toml cthuwu/Cargo.lock ./
COPY cthuwu/src ./src
RUN cargo build --locked --release

FROM node:22-bookworm-slim AS runtime
WORKDIR /data
RUN mkdir -p /opt/cthuwu/agent /data && chown -R node:node /opt/cthuwu /data
COPY --from=rust-build /build/cthuwu/target/release/uwubot /usr/local/bin/uwubot
COPY --from=agent-build --chown=node:node /build/agent/dist /opt/cthuwu/agent/dist
COPY --from=agent-build --chown=node:node /build/agent/node_modules /opt/cthuwu/agent/node_modules
COPY --from=agent-build --chown=node:node /build/agent/package.json /opt/cthuwu/agent/package.json

ENV UWUBOT_DATA_DIR=/data \
    UWUBOT_SIDECAR=/opt/cthuwu/agent/dist/index.js \
    UWUBOT_XMTP_ENV=dev

USER node
VOLUME ["/data"]
ENTRYPOINT ["/usr/local/bin/uwubot"]

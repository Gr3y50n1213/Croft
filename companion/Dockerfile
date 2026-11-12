# This Source Code Form is subject to the terms of the GNU Affero General Public License,
# version 3 or later. See LICENSE-AGPL-3.0 at the repository root.
#
# The Croft Companion. Build from the repository root:
#   docker build -f companion/Dockerfile -t croft-companion .
# The web app (T30): a static build, served by the Companion.
FROM node:22-bookworm-slim AS web
WORKDIR /src
COPY design/generated/ design/generated/
COPY web/ web/
RUN cd web && npm ci && npm run build

FROM rust:1-slim-bookworm AS build
# SQLCipher (users' mail databases, T26) links against the system's OpenSSL.
RUN apt-get update && apt-get install -y --no-install-recommends libssl-dev pkg-config \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /src
# The Companion uses the core (and its patched crates) by path.
COPY core/ core/
COPY companion/ companion/
RUN cd companion && cargo build --release --locked

FROM debian:bookworm-slim
# OpenSSL's runtime library for SQLCipher.
RUN apt-get update && apt-get install -y --no-install-recommends libssl3 \
    && rm -rf /var/lib/apt/lists/*
# The master key lives on its own volume (/keys), never next to the database (/data): one stolen
# backup must not hold both. Both folders belong to the Companion's user only.
RUN useradd --system --uid 10001 --home-dir /data --shell /usr/sbin/nologin croft \
    && mkdir -p /data /keys && chown croft /data /keys && chmod 700 /data /keys
COPY --from=build /src/companion/target/release/croft-companion /usr/local/bin/croft-companion
COPY --from=web /src/web/build /usr/share/croft/web
USER croft
VOLUME ["/data", "/keys"]
ENV CROFT_COMPANION_DATA_DIR=/data CROFT_COMPANION_MASTER_KEY_FILE=/keys/master.key CROFT_COMPANION_LISTEN=0.0.0.0:8480 \
    CROFT_COMPANION_WEB_DIR=/usr/share/croft/web
EXPOSE 8480
HEALTHCHECK --interval=30s --timeout=5s CMD ["/usr/local/bin/croft-companion", "healthcheck"]
ENTRYPOINT ["/usr/local/bin/croft-companion"]
CMD ["serve"]

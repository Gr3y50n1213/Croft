# This Source Code Form is subject to the terms of the GNU Affero General Public License,
# version 3 or later. See LICENSE-AGPL-3.0 at the repository root.
#
# The Croft Companion. Build from the repository root:
#   docker build -f companion/Dockerfile -t croft-companion .
FROM rust:1-slim-bookworm AS build
WORKDIR /src
COPY companion/ companion/
RUN cd companion && cargo build --release --locked

FROM debian:bookworm-slim
RUN useradd --system --uid 10001 --home-dir /data --shell /usr/sbin/nologin croft \
    && mkdir -p /data && chown croft /data && chmod 700 /data
COPY --from=build /src/companion/target/release/croft-companion /usr/local/bin/croft-companion
USER croft
VOLUME ["/data"]
ENV CROFT_COMPANION_DATA_DIR=/data CROFT_COMPANION_LISTEN=0.0.0.0:8480
EXPOSE 8480
HEALTHCHECK --interval=30s --timeout=5s CMD ["/usr/local/bin/croft-companion", "healthcheck"]
ENTRYPOINT ["/usr/local/bin/croft-companion"]
CMD ["serve"]

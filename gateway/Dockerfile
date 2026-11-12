# This Source Code Form is subject to the terms of the GNU Affero General Public License,
# version 3 or later. See LICENSE-AGPL-3.0 at the repository root.
#
# The Croft push gateway. Build from the repository root:
#   docker build -f gateway/Dockerfile -t croft-push-gateway .
FROM rust:1-slim-bookworm AS build
WORKDIR /src
# The gateway uses the core's HTTPS client (and its patched crate) by path.
COPY core/ core/
COPY gateway/ gateway/
RUN cd gateway && cargo build --release --locked

FROM debian:bookworm-slim
# The gateway's key and the service account key live on their own read-only volume (/keys).
RUN useradd --system --uid 10002 --no-create-home --shell /usr/sbin/nologin croft \
    && mkdir -p /keys && chown croft /keys && chmod 700 /keys
COPY --from=build /src/gateway/target/release/croft-push-gateway /usr/local/bin/croft-push-gateway
USER croft
VOLUME ["/keys"]
ENV CROFT_GATEWAY_LISTEN=0.0.0.0:8490 CROFT_GATEWAY_KEY_FILE=/keys/gateway.key \
    CROFT_GATEWAY_SERVICE_ACCOUNT_FILE=/keys/service-account.json
EXPOSE 8490
HEALTHCHECK --interval=30s --timeout=5s CMD ["/usr/local/bin/croft-push-gateway", "healthcheck"]
ENTRYPOINT ["/usr/local/bin/croft-push-gateway"]

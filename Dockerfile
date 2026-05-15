# Dockerfile used by GoReleaser (see apps/p2proxy/.goreleaser.yaml).
# GoReleaser invokes `docker buildx build --platform=linux/<arch>` inside the
# `dist/p2proxy_linux_<arch>/` directory (or similar) where the pre-built
# binaries already live. We just copy them into a minimal runtime image.
#
# Do NOT build p2proxy from source here — the binary in this Dockerfile's
# context was cross-compiled by the parent GitLab CI job. Building from
# source would require the full monorepo workspace, which isn't present
# in GoReleaser's per-arch build context.

FROM debian:trixie-slim

RUN apt-get update && \
    apt-get install -y --no-install-recommends ca-certificates && \
    rm -rf /var/lib/apt/lists/*

COPY p2proxy /usr/local/bin/p2proxy

EXPOSE 1080
EXPOSE 45445/udp

# TTY rendering inside a container is useless. Default to headless mode;
# override by passing `-e NO_UI=false` if running with a TTY attached.
ENV NO_UI=true

CMD ["/usr/local/bin/p2proxy"]

ARG VERSION
ARG TARGETARCH

FROM alpine:3.20 AS download
ARG VERSION
ARG TARGETARCH
RUN apk add --no-cache curl tar xz ca-certificates && \
    curl -fsSL --retry 5 --retry-delay 5 -o /tmp/p2p.tar.xz \
      "https://github.com/BitpingApp/p2proxy/releases/download/v${VERSION}/p2proxy-${VERSION}-linux-${TARGETARCH}.tar.xz" && \
    mkdir -p /tmp/extract && \
    tar -xJf /tmp/p2p.tar.xz -C /tmp/extract && \
    mv /tmp/extract/p2proxy-${VERSION}-linux-${TARGETARCH}/p2proxy /tmp/p2proxy

FROM gcr.io/distroless/cc-debian12:nonroot
COPY --from=download --chmod=755 /tmp/p2proxy /usr/local/bin/p2proxy
EXPOSE 1080
EXPOSE 45445/udp
ENV NO_UI=true
ENTRYPOINT ["/usr/local/bin/p2proxy"]

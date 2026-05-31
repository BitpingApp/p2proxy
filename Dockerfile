FROM gcr.io/distroless/cc-debian12:nonroot
COPY --chmod=755 p2proxy /usr/local/bin/p2proxy
EXPOSE 1080
EXPOSE 45445/udp
ENV NO_UI=true
ENTRYPOINT ["/usr/local/bin/p2proxy"]

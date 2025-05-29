FROM debian:trixie-slim

RUN apt-get update && apt-get install -y \
    ca-certificates \
    curl \
    xz-utils \
    && rm -rf /var/lib/apt/lists/*

# Download and run the installer
RUN curl --proto '=https' --tlsv1.2 -LsSf https://github.com/BitpingApp/p2proxy/releases/latest/download/p2proxy-installer.sh -o installer.sh

RUN chmod +x installer.sh

RUN ./installer.sh

WORKDIR /app

# Move the binary to a location in PATH
RUN mv ~/.cargo/bin/p2proxy /app/p2proxy

EXPOSE 1080 45445
CMD ["/app/p2proxy"]
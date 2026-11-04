# Build stage
FROM rust:1.96-slim-bookworm AS builder

# Install system dependencies
RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config \
    libssl-dev \
    git \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /usr/src/syneroym

# Copy the entire workspace
COPY . .

# Build release binaries
RUN cargo build --release -p syneroym-substrate -p roymctl

# Final stage
FROM debian:bookworm-slim

# Install runtime dependencies (e.g. ca-certificates for registry communication)
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# Copy binaries from builder
COPY --from=builder /usr/src/syneroym/target/release/syneroym-substrate /usr/local/bin/
COPY --from=builder /usr/src/syneroym/target/release/roymctl /usr/local/bin/

# Default ports:
# 7964 - Iroh HTTP info port
# 7965 - Iroh QUIC port (UDP)
# 7961 - Community Registry http port
# 7960 - Client Gateway http port. The gateway binds 127.0.0.1 unless
#        [roles.client_gateway.remote_access] is set, and no port mapping can
#        reach a loopback address inside a container. Mount a config that sets
#        it (see deploy/config.community.example.toml).
#
# `run` with no --config is a development setup. Mount a config for a real node.
EXPOSE 7964 7965/udp 7961 7960

ENTRYPOINT ["syneroym-substrate"]
CMD ["run"]

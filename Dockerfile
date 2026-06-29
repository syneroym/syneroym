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
# 7965 - Iroh QUIC port
# 7961 - Community Registry http port
# 7960 - Client Gateway http port
EXPOSE 7964 7965 7961 7960

ENTRYPOINT ["syneroym-substrate"]
CMD ["run"]

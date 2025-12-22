# syntax=docker/dockerfile:1

# Minimal runtime image with pre-built otto binary
# Binary is provided via build context from CI artifacts

FROM debian:bookworm-slim

# Install minimal runtime dependencies
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && apt-get clean

# Create non-root user for security
RUN useradd --create-home --shell /bin/bash otto

# Copy pre-built binary from build context
COPY --from=binary otto /usr/local/bin/otto

# Ensure binary is executable
RUN chmod +x /usr/local/bin/otto

# Switch to non-root user
USER otto
WORKDIR /home/otto

# Verify installation
RUN otto --version

ENTRYPOINT ["otto"]
CMD ["--help"]

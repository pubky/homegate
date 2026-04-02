# ========================
# Build Stage
# ========================
FROM rust:1.89.0-alpine3.20 AS builder

# Install build dependencies, including static OpenSSL libraries
RUN apk add --no-cache \
    pkgconfig \
    build-base \
    curl

# Set the working directory
WORKDIR /usr/src/app

# Copy over Cargo.toml and Cargo.lock for dependency caching
COPY Cargo.toml Cargo.lock ./

# Copy over all the source code
COPY . .

# Build the project in release mode for the MUSL target
RUN cargo build --release --bin homegate

# Strip the binary to reduce size
RUN strip target/release/homegate

# ========================
# Runtime Stage
# ========================
FROM alpine:3.20

# Install runtime dependencies (only ca-certificates)
RUN apk add --no-cache ca-certificates

# Copy the compiled binary from the builder stage
COPY --from=builder /usr/src/app/target/release/homegate /usr/local/bin/homegate

# Set the working directory
WORKDIR /usr/local/bin

# Expose the port the homegate listens on (should match that of config.toml)
EXPOSE 8080

# Set the default command to run the binary
CMD ["homegate"]

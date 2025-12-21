# Build from Rust Alpine for smaller image size
FROM rust:1.75-alpine as builder

# Install build dependencies
RUN apk add --no-cache musl-dev pkgconfig openssl-dev

WORKDIR /app

# Copy manifests
COPY Cargo.toml Cargo.lock ./

# Copy source code
COPY src ./src

# Build release binary
RUN cargo build --release --target x86_64-unknown-linux-musl
RUN strip target/x86_64-unknown-linux-musl/release/ffmpeg-video-processor

# Final stage
FROM alpine:latest

# Install FFmpeg and other runtime dependencies
RUN apk add --no-cache ffmpeg libgcc

# Create app user
RUN addgroup -g 1000 app && \
    adduser -D -u 1000 -G app app

WORKDIR /app

# Copy binary from builder
COPY --from=builder /app/target/x86_64-unknown-linux-musl/release/ffmpeg-video-processor /usr/local/bin/

# Copy default config
COPY config.example.toml /etc/ffmpeg-video-processor/config.toml

# Create directories for input/output
RUN mkdir -p /input /output && \
    chown -R app:app /input /output

USER app

VOLUME ["/input", "/output"]

ENTRYPOINT ["ffmpeg-video-processor"]
CMD ["--help"]

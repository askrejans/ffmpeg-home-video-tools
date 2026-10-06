# Build
FROM rust:1.99-alpine AS builder
RUN apk add --no-cache musl-dev
WORKDIR /app
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY src ./src
COPY templates ./templates
RUN cargo build --release --locked --no-default-features --features cli,builtin-templates

# Run
FROM alpine:3
RUN apk add --no-cache ffmpeg font-noto \
    && addgroup -g 1000 app \
    && adduser -D -u 1000 -G app app \
    && mkdir -p /input /output \
    && chown app:app /input /output
COPY --from=builder /app/target/release/ffmpeg-video-processor /usr/local/bin/
USER app
VOLUME ["/input", "/output"]
ENTRYPOINT ["ffmpeg-video-processor"]
CMD ["--help"]

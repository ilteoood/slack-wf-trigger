# syntax=docker/dockerfile:1.7

FROM --platform=$BUILDPLATFORM rust:1.85-alpine AS builder
ARG TARGETARCH
RUN rustup target add ${TARGETARCH}-unknown-linux-musl
RUN apk add --no-cache musl-dev ca-certificates
WORKDIR /build
COPY Cargo.toml Cargo.lock ./
RUN mkdir -p src \
    && echo 'fn main(){}' > src/main.rs \
    && cargo build --release --target ${TARGETARCH}-unknown-linux-musl \
    && rm -rf src target/${TARGETARCH}-unknown-linux-musl/release/deps/slack_wf_trigger*
COPY src ./src
RUN touch src/main.rs \
    && cargo build --release --target ${TARGETARCH}-unknown-linux-musl
RUN cp target/${TARGETARCH}-unknown-linux-musl/release/slack-wf-trigger /slack-wf-trigger \
    && strip /slack-wf-trigger

FROM --platform=$TARGETPLATFORM alpine:3.20
COPY --from=builder /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/ca-certificates.crt
COPY --from=builder /slack-wf-trigger /slack-wf-trigger
RUN addgroup -g 65534 nobody \
    && adduser -u 65534 -G nobody -D -H nobody \
    && mkdir -p /etc/slack-wf-trigger /var/lib/slack-wf-trigger \
    && chown -R nobody:nobody /var/lib/slack-wf-trigger /etc/slack-wf-trigger
USER nobody
WORKDIR /var/lib/slack-wf-trigger
ENV PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin
ENV SSL_CERT_FILE=/etc/ssl/certs/ca-certificates.crt \
    RUST_LOG=info \
    SLACK_WF_TRIGGER_CONFIG=/etc/slack-wf-trigger/rules.json \
    SLACK_WF_TRIGGER_POLL_INTERVAL=10
ENTRYPOINT ["/slack-wf-trigger"]
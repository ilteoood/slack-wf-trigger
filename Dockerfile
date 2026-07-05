# syntax=docker/dockerfile:1.7

FROM --platform=$BUILDPLATFORM rust:1.96.1-alpine3.24 AS builder
ARG TARGETARCH
RUN case "${TARGETARCH}" in \
      amd64) rust_target=x86_64-unknown-linux-musl ;; \
      arm64) rust_target=aarch64-unknown-linux-musl ;; \
      *) echo "Unsupported TARGETARCH: ${TARGETARCH}" >&2; exit 1 ;; \
    esac \
    && echo "RUST_TARGET=${rust_target}" > /tmp/rt.env \
    && rustup target add "${rust_target}"
RUN apk add --no-cache musl-dev ca-certificates
WORKDIR /build
COPY Cargo.toml Cargo.lock ./
RUN . /tmp/rt.env \
    && mkdir -p src \
    && echo 'fn main(){}' > src/main.rs \
    && cargo build --release --target "${RUST_TARGET}" \
    && rm -rf src target/"${RUST_TARGET}"/release/deps/slack_wf_trigger*
COPY src ./src
RUN . /tmp/rt.env \
    && touch src/main.rs \
    && cargo build --release --target "${RUST_TARGET}"
RUN . /tmp/rt.env \
    && cp target/"${RUST_TARGET}"/release/slack-wf-trigger /slack-wf-trigger \
    && strip /slack-wf-trigger

FROM --platform=$TARGETPLATFORM alpine:3.20
COPY --from=builder /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/ca-certificates.crt
COPY --from=builder /slack-wf-trigger /slack-wf-trigger
RUN mkdir -p /etc/slack-wf-trigger /var/lib/slack-wf-trigger \
    && chown -R nobody:nobody /var/lib/slack-wf-trigger /etc/slack-wf-trigger
USER nobody
WORKDIR /var/lib/slack-wf-trigger
ENV PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin
ENV SSL_CERT_FILE=/etc/ssl/certs/ca-certificates.crt \
    RUST_LOG=info \
    SLACK_WF_TRIGGER_CONFIG=/etc/slack-wf-trigger/rules.json \
    SLACK_WF_TRIGGER_POLL_INTERVAL=10
ENTRYPOINT ["/slack-wf-trigger"]
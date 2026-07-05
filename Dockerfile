# syntax=docker/dockerfile:1.7

FROM --platform=$TARGETPLATFORM alpine:3.20

ARG TARGETARCH

RUN apk add --no-cache ca-certificates
COPY --chmod=0755 bin/slack-wf-trigger-${TARGETARCH} /slack-wf-trigger

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
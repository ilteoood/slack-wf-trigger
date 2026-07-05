FROM --platform=$TARGETPLATFORM alpine:3.20

ARG TARGETARCH

RUN apk add --no-cache ca-certificates

COPY --chmod=0755 bin/slack-wf-trigger-${TARGETARCH} /slack-wf-trigger

RUN mkdir -p /etc/slack-wf-trigger /var/lib/slack-wf-trigger \
    && chown -R nobody:nogroup /etc/slack-wf-trigger /var/lib/slack-wf-trigger

USER nobody
WORKDIR /var/lib/slack-wf-trigger

ENV RUST_LOG=info
ENV SSL_CERT_FILE=/etc/ssl/certs/ca-certificates.crt
ENV SLACK_WF_TRIGGER_CONFIG=/etc/slack-wf-trigger/rules.json
ENV SLACK_WF_TRIGGER_POLL_INTERVAL=10

CMD ["/slack-wf-trigger"]

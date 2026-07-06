FROM alpine:latest AS builder
ARG TARGETARCH
WORKDIR /builder
COPY . .
RUN ./scripts/binary.sh $TARGETARCH && \
    echo "nobody:x:65534:65534:Nobody:/:" > /etc_passwd

FROM scratch
COPY --from=builder --chmod=755 /builder/slack-wf-trigger ./slack-wf-trigger
COPY --from=builder "/etc_passwd" "/etc/passwd"
COPY --from=builder /etc/ssl/certs/ca-certificates.crt /usr/local/ssl/ca-certificates.crt
USER nobody

ENV SSL_CERT_FILE=/usr/local/ssl/ca-certificates.crt
ENV RUST_LOG=info
ENV SLACK_WF_TRIGGER_CONFIG=/etc/slack-wf-trigger/rules.json
ENV SLACK_WF_TRIGGER_POLL_INTERVAL=10

ENV SLACK_BASE_URL=https://slack.com
ENV SLACK_TOKEN=your-slack-token
ENV SLACK_COOKIE=your-slack-cookie

CMD ["./slack-wf-trigger"]
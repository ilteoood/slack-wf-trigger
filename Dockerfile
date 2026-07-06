FROM alpine:latest AS builder
ARG TARGETARCH
WORKDIR /builder
COPY . .
RUN ./scripts/binary.sh $TARGETARCH

FROM alpine:latest
COPY --from=builder --chmod=755 /builder/slack-wf-trigger ./slack-wf-trigger

RUN apk add --no-cache curl

ENV SSL_CERT_FILE=/etc/ssl/certs/ca-certificates.crt
ENV RUST_LOG=info
ENV SLACK_WF_HOME=/root/slack-wf-trigger
ENV SLACK_WF_TRIGGER_POLL_INTERVAL=10

ENV SLACK_BASE_URL=https://slack.com
ENV SLACK_TOKEN=your-slack-token
ENV SLACK_COOKIE=your-slack-cookie

CMD ["./slack-wf-trigger"]
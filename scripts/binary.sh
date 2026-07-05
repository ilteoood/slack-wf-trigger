#!/bin/sh

case "$1" in

  amd64)
    VARIANT=x86_64-unknown-linux-musl
    ;;

  arm64)
    VARIANT=aarch64-unknown-linux-musl
    ;;

  *)
    echo "unsupported arch: $1" >&2
    exit 1
    ;;
esac

mv ./slack-wf-trigger-${VARIANT}/slack-wf-trigger ./slack-wf-trigger
# slack-wf-trigger

A long-running Rust daemon that watches Slack channels and runs shell commands
when incoming messages match a configurable pattern. It is the trigger
counterpart to `listening-to`: instead of pushing state out to Slack, it pulls
state in from Slack to drive local side effects (deployments, webhooks, ops
scripts).

The binary connects to one Slack workspace via Web API polling
(`conversations.history`), evaluates every new message against a JSON rule
list, and runs the matched rule's command via `sh -c`. Per-channel cursors are
persisted to disk so reconnects and restarts do not re-trigger commands.

## Highlights

- Single static binary, no runtime install steps.
- Substring or regex matching against the message text (Slack mrkdwn is
  stripped before matching).
- Configurable per-rule shell command with the originating message context
  exposed as environment variables.
- Idempotent resume via per-channel `ts` cursors persisted to JSON.
- Slack reactions as status: `:thumbsup:` on match, `:white_check_mark:` on
  success, `:x:` on failure. Messages authored by the daemon's own user are
  processed like any other.
- Graceful shutdown on `SIGINT` / `SIGTERM`.

> [!WARNING]
> Triggered commands are evaluated by the same shell that runs the binary —
> no sandboxing. Run this only on a trusted host as a non-root user and only
> with rules you authored.

## Slack token requirements

The daemon authenticates with a **user token** (`xoxp-...`) because it polls
the Web API from inside a workspace. The required scopes are:

| Scope | Purpose | Required |
|---|---|---|
| `channels:history`, `groups:history` | Read messages from public and private channels | yes |
| `channels:read`, `groups:read` | Resolve channel names to IDs | yes |
| `reactions:write` | Add `:thumbsup:` / `:white_check_mark:` / `:x:` | yes |
| `users:read` | Resolve user metadata on matching messages | yes |
| `im:history`, `mpim:history`, `im:read`, `mpim:read` | Read DMs and group DMs | optional |

## Configuration

Configuration is split between CLI flags (also overridable by env vars) and a
`rules.json` file in the home directory.

### CLI flags / environment variables

| Flag | Env var | Default | Description |
|---|---|---|---|
| `--home <DIR>` | `SLACK_WF_HOME` | — (required) | Directory containing `rules.json`. The cursor file lives here too. |
| `--slack-token <TOKEN>` | `SLACK_TOKEN` | — | Slack user token (`xoxp-...`). Marked `hide_env` in `--help`. |
| `--slack-cookie <COOKIE>` | `SLACK_COOKIE` | unset | Optional `d=` cookie value used as a fallback auth header. |
| `--slack-base-url <URL>` | `SLACK_BASE_URL` | `https://slack.com/api` | Slack Web API base URL (override for a proxy or a Slack-compatible server). |
| `--poll-interval <SECS>` | `SLACK_WF_TRIGGER_POLL_INTERVAL` | `10` | Seconds between poll cycles. Minimum `1`. |

CLI flags win over the matching env var. Every flag except `--home` is
optional when its env var is set; `--home` has no default and the daemon
refuses to start without it.

### `rules.json`

`<SLACK_WF_HOME>/rules.json` is a JSON array of rule objects:

```json
[
  {
    "channel": "deploys",
    "message": "deploy prod",
    "command": "curl -fsS -X POST https://ci.example.com/deploy/prod"
  },
  {
    "channel": "alerts",
    "message": "^ERROR (.+)$",
    "regex": true,
    "command": "logger -t slack-wf-trigger -p user.err -- 'matched: $SLACK_WF_TRIGGER_TEXT'"
  }
]
```

| Field | Type | Required | Description |
|---|---|---|---|
| `channel` | string | yes | Channel name (e.g. `general`) or Slack channel ID (e.g. `C0123ABCD`). The daemon must be able to see it. |
| `message` | string | yes | Substring (default) or regex pattern (when `regex: true`) matched against the message text. |
| `regex` | bool | no | When `true`, `message` is compiled as a Rust regex. |
| `command` | string | yes | Shell command line, passed verbatim to `sh -c`. |

The daemon fails fast if the file is missing, malformed, not a JSON array,
contains a rule with empty fields, or references a channel the authenticated
user cannot see.

### Home directory layout

```
SLACK_WF_HOME/
├── rules.json                            # input, read once at startup
└── .slack-wf-trigger.cursors.json        # managed by the daemon
```

`.slack-wf-trigger.cursors.json` is rewritten atomically after every
successful poll cycle. On the first run (no cursor file) the daemon seeds
each channel's cursor from its initial poll and **does not** trigger any
commands.

### Environment passed to triggered commands

When a command is spawned, the daemon sets the working directory to
`SLACK_WF_HOME` and exports:

| Variable | Value |
|---|---|
| `SLACK_WF_TRIGGER_CHANNEL` | Channel name if known, else the channel ID. |
| `SLACK_WF_TRIGGER_CHANNEL_ID` | Slack channel ID. |
| `SLACK_WF_TRIGGER_USER` | User ID (`U…`) of the message author, when present. |
| `SLACK_WF_TRIGGER_TEXT` | Raw message text (mrkdwn **not** stripped). |
| `SLACK_WF_TRIGGER_TS` | Slack message timestamp. |
| `SLACK_WF_TRIGGER_RULE_INDEX` | Zero-based index of the matched rule. |

stdout and stderr of the child are captured (last 4 KiB each) and emitted as
`tracing` log lines.

## Running

### Local binary

```sh
export SLACK_TOKEN=xoxp-...
export SLACK_WF_HOME=$HOME/.config/slack-wf-trigger

mkdir -p "$SLACK_WF_HOME"
# drop a rules.json in there, see example above

cargo run --release -- \
  --home "$SLACK_WF_HOME" \
  --slack-token "$SLACK_TOKEN"
```

Or with `cargo install` once the binary is published; flags and env vars are
identical.

### Docker

The official image is published as `ilteoood/slack-wf-trigger` (multi-arch:
`linux/amd64`, `linux/arm64`). A `docker-compose.yml` is included in the
repo:

```yaml
services:
  slack-wf-trigger:
    image: ilteoood/slack-wf-trigger:latest
    container_name: slack-wf-trigger
    restart: unless-stopped
    environment:
      SLACK_TOKEN: ${SLACK_TOKEN:?SLACK_TOKEN must be set}
      SLACK_COOKIE: ${SLACK_COOKIE:?SLACK_COOKIE must be set}
      SLACK_BASE_URL: ${SLACK_BASE_URL:-https://slack.com}
      SLACK_WF_HOME: /var/lib/slack-wf-trigger
    volumes:
      - ./rules.json:/var/lib/slack-wf-trigger/rules.json:ro
      - slack-wf-trigger-state:/var/lib/slack-wf-trigger

volumes:
  slack-wf-trigger-state:
```

Run it with:

```sh
docker compose up -d
```

The named volume `slack-wf-trigger-state` persists
`.slack-wf-trigger.cursors.json` across restarts.

> [!NOTE]
> The container ships with `SLACK_COOKIE` listed as required in the bundled
> compose file because Slack's Web API increasingly requires the `d=` cookie
> alongside the bearer token. The binary itself treats the cookie as
> optional; drop the line if your workspace still accepts the bare bearer
> token.

## Logging

Logs go to stdout and are filtered with `RUST_LOG` (default `info`). Useful
levels:

```sh
RUST_LOG=info,slack_wf_trigger=debug ./slack-wf-trigger
```

Each poll cycle, command spawn, and command exit is emitted as a structured
`tracing` event with channel, rule index, exit code, duration, and tail of
stdout/stderr.

## Development

Requirements: Rust `1.96` or newer, edition 2024. The CI matrix targets
`x86_64-unknown-linux-musl` and `aarch64-unknown-linux-musl`.

```sh
cargo fmt --all
cargo clippy --all-targets --locked -- -D warnings
cargo test
```

Tests use `wiremock` for the Slack surface and `tempfile` for cursor
round-trips; integration coverage lives in `tests/integration.rs`.

## Project layout

```
src/
├── main.rs       # clap CLI, signal wiring
├── lib.rs        # poll loop, channel resolution
├── config.rs     # rules.json loader and validation
├── cursors.rs    # atomic JSON cursor store
├── slack.rs      # minimal Slack Web API client (4 endpoints)
├── trigger.rs    # match, run, react orchestration
└── mrkdwn.rs     # Slack mrkdwn → plain text stripper
spec/             # design specifications (tool + docker image)
tests/            # integration tests
scripts/          # build helpers used by the Dockerfile
```

See `spec/spec-tool-slack-channel-watcher.md` for the full functional
specification and `spec/spec-tool-slack-channel-watcher-docker.md` for the
image contract.
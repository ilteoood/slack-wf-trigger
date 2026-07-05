---
title: Slack Channel Watcher — Specification
version: 0.1.0
date_created: 2026-07-05
last_updated: 2026-07-05
owner: ilteoood
tags: [tool, slack, automation, rust]
---

# Introduction

This document specifies a Rust command-line application, `wf-trigger`, that watches configured Slack channels for messages matching configured text patterns and executes configured shell commands when a match is observed. The application is the trigger counterpart to `listening-to`: instead of pushing state out to Slack, it pulls state in from Slack to drive local side effects (e.g. webhook calls, deployments, scripts).

The tool is intended to be a long-running single-workspace process running on a developer machine or small server.

## 1. Purpose & Scope

### Purpose

Provide a single static binary that:

- Connects to a single Slack workspace via Web API polling (`conversations.history`) using a user token.
- Evaluates incoming messages against a user-defined JSON rule list.
- Executes a user-defined shell command for each matching message.
- Deduplicates messages by Slack `ts` so that reconnection or replay does not retrigger commands.

### Scope

**In scope:**

- One Slack workspace.
- One process per workspace (multi-workspace is achieved by running multiple instances).
- Plain text and substring matching (regex optional).
- Synchronous execution of one command per matching message (sequentially per rule; multiple rules may run concurrently if matched by independent messages).
- Structured logging via `tracing`.
- Configuration loaded from a JSON file at startup.

**Out of scope (v1):**

- Hot reload of the configuration file (a restart is required to pick up changes).
- Reactions / threads / ephemeral replies back to Slack from the triggered command, beyond the three status reactions (`thumbsup`, `white_check_mark`, `x`) defined in REQ-011 through REQ-014.
- Interactive prompts, multi-account token stores, OAuth flows beyond reading pre-provisioned tokens.
- Guaranteed at-least-once delivery to commands (best-effort; see Section 3 for the durability trade-off).

### Audience

- `ilteoood` (the operator). This is a single-user tool. Multi-tenant hardening is not a goal.

## 2. Definitions

| Term | Definition |
|---|---|
| Rule | One entry in the JSON config: `{ channel, message, command }`. |
| Match | A message whose `text` contains the rule's `message` value per the rule's match mode. |
| Trigger | The act of running the rule's `command` once per match. |
| `ts` | Slack's per-message timestamp identifier; unique within a channel. |
| Cursor | The newest `ts` the binary has fully processed in a channel; persisted to disk so restarts resume from there. |
| Events API | Slack's HTTP webhook-based push channel; requires a public HTTPS endpoint. Out of scope for v1. |
| Web API polling | Repeatedly calling `conversations.history` on a `tokio::time::interval`. Used by `wf-trigger`. |
| User token | Slack token of form `xoxp-...` representing a human user. The only token `wf-trigger` uses. |

## 3. Requirements, Constraints & Guidelines

### Functional Requirements

- **REQ-001**: The binary shall load its rule list from a JSON file path provided via `--config <path>` CLI flag or `WF_TRIGGER_CONFIG` env var (CLI flag wins).
- **REQ-002**: The rule list shall be a JSON array. Each element must contain `channel` (string), `message` (string), `command` (string).
- **REQ-003**: On startup the binary shall establish a Slack connection and begin receiving messages from the union of channels named in the rule list.
- **REQ-004**: For every received message, the binary shall evaluate each rule whose `channel` matches the message's channel.
- **REQ-005**: A rule matches when the rule's `message` value is a substring of the message's `text` (case-sensitive, after stripping Slack mrkdwn formatting). Optional `regex: true` per rule switches the rule to regex matching against `text`.
- **REQ-006**: For each match, the binary shall spawn the rule's `command` via `sh -c` with the working directory set to the directory containing the config file.
- **REQ-007**: The binary shall expose the originating message metadata to the command via environment variables: `WF_TRIGGER_CHANNEL`, `WF_TRIGGER_USER`, `WF_TRIGGER_TEXT`, `WF_TRIGGER_TS`, `WF_TRIGGER_RULE_INDEX`.
- **REQ-008**: The binary shall persist per-channel cursors (`latest` `ts` per channel) to `<config-dir>/.wf-trigger.cursors.json` after every successful poll cycle. On startup the binary shall resume each channel from its persisted cursor. First run (no cursor file) shall fetch the most recent 100 messages per channel without triggering commands and seed the cursor.
- **REQ-009**: The binary shall emit a structured log line on every: rule match, command spawn, command exit (with exit code, duration, stdout/stderr tail).
- **REQ-010**: The binary shall exit non-zero on startup if the config file is missing, malformed, or references no resolvable channels.
- **REQ-011**: When a message matches at least one rule, the binary shall add a `thumbsup` reaction to that message **before** spawning the first matching command. Failure to add the reaction shall be logged but shall not block command execution.
- **REQ-012**: When a triggered command exits with code `0`, the binary shall add a `white_check_mark` reaction to the originating message.
- **REQ-013**: When a triggered command exits with any non-zero code (including signals), the binary shall add an `x` reaction to the originating message.
- **REQ-014**: The binary shall never react to messages authored by the authenticated user (no self-reactions). All reaction additions shall be idempotent: Slack's `already_reacted` error shall be treated as success.

### Security Requirements

- **SEC-001**: Commands are evaluated by the same shell that runs the binary; no sandboxing is applied. The operator is trusted. This must be documented in `--help`.
- **SEC-002**: The Slack user token shall be read from the `SLACK_USER_TOKEN` env var only. It shall never be written to logs, the cursors file, or any other on-disk artifact.
- **SEC-003**: Command stdout and stderr shall be truncated to 4 KiB before logging.
- **SEC-004**: The JSON config file shall be parsed with a depth limit of 32 to reject pathological inputs.

### Constraints

- **CON-001**: Rust stable toolchain (latest, ≥ 1.85). Edition 2024.
- **CON-002**: Single binary, no runtime install steps. Distributed via `cargo install` or pre-built release tarball.
- **CON-003**: Slack connection strategy shall be **Web API polling** (`conversations.history`) because the operator provisions a user token only. No app-level token, no Socket Mode, no public HTTPS endpoint.
- **CON-004**: Linux x86_64 and aarch64 are the supported targets. macOS is best-effort.

### Guidelines

- **GUD-001**: Use `tokio` as the async runtime; `tracing` + `tracing-subscriber` for logs; `serde` + `serde_json` for the config and cursors file; `reqwest` (JSON feature only) for Slack Web API calls. No Slack SDK crate needed — the surface area is four endpoints.
- **GUD-002**: Keep dependencies minimal. Avoid pulling in `notify` for hot reload in v1.
- **GUD-003**: Prefer `&str` over `String` in public APIs and free functions.
- **GUD-004**: No `unwrap` outside `main` and tests; use `anyhow::Result` for error propagation.

## 4. Interfaces & Data Contracts

### CLI

```
wf-trigger --config /etc/wf-trigger/rules.json
```

| Flag | Env | Default | Purpose |
|---|---|---|---|
| `--config <PATH>` | `WF_TRIGGER_CONFIG` | none (required) | Path to the JSON rule list. |
| `--poll-interval <SECS>` | `WF_TRIGGER_POLL_INTERVAL` | `10` | Seconds between polls per channel. Must be ≥ 1. |

Exit codes:

| Code | Meaning |
|---|---|
| `0` | Clean shutdown via SIGINT / SIGTERM. |
| `1` | Startup failure (config, auth, network). |
| `2` | Runtime panic. |

### Config JSON Schema

```jsonc
[
  {
    "channel": "general",          // required, string. Channel name OR channel ID (Cxxxxxxx).
    "message": "deploy prod",      // required, string. Substring or regex source.
    "command": "curl -X POST https://ci.example.com/deploy", // required, string. Passed to `sh -c`.
    "regex": false                  // optional, bool, default false. Switch `message` from substring to regex.
  }
]
```

Validation rules:

- `channel` non-empty.
- `message` non-empty. If `regex: true`, must compile as a valid `regex::Regex`.
- `command` non-empty.

### Cursor File

`<config-dir>/.wf-trigger.cursors.json`:

```json
{
  "C0123": "1717600042.000456",
  "C0456": "1717600100.000789"
}
```

This is a `HashMap<ChannelId, LatestTs>`. Written atomically (write to `.tmp`, rename) after every successful poll cycle so that a restart resumes from the last fully-processed message.

### Environment Variables Injected Into Triggered Commands

| Var | Source |
|---|---|
| `WF_TRIGGER_CHANNEL` | Message channel name (resolved from id if available). |
| `WF_TRIGGER_CHANNEL_ID` | Message channel id. |
| `WF_TRIGGER_USER` | Message user id (or bot id if posted by a bot). |
| `WF_TRIGGER_TEXT` | Raw `text` field of the message. |
| `WF_TRIGGER_TS` | Message `ts`. |
| `WF_TRIGGER_RULE_INDEX` | Zero-based index of the matched rule in the config array. |

The command inherits the parent process environment except that these six are added/overwritten.

### Slack Scopes Required

The token is a **user token** (`xoxp-...`). All scopes below are user scopes.

| Scope | Reason |
|---|---|
| `channels:history` | Read messages from public channels the user is in. |
| `groups:history` | Read messages from private channels the user is in. |
| `im:history`, `mpim:history` | Optional; needed only if rules target DMs / group DMs. |
| `channels:read` | Resolve public channel names to IDs at startup. |
| `groups:read` | Resolve private channel names to IDs at startup. |
| `im:read`, `mpim:read` | Optional; resolve DM / group DM names to IDs. |
| `reactions:write` | Add `thumbsup`, `white_check_mark`, `x` reactions. |
| `users:read` | Resolve the authenticated user's ID for self-message filtering. |

The operator installs (re-installs) the Slack app once to mint the user token with these scopes, exports it as `SLACK_USER_TOKEN`, and starts the binary. No bot user, no app-level token, no Socket Mode, no public URL.

## 5. Acceptance Criteria

- **AC-001**: Given a config with one rule `{channel: "C0", message: "ping", command: "echo pong"}`, when a message with text `"please ping me"` arrives in `C0`, then a single `echo pong` process is spawned, the cursor for `C0` advances past that message's `ts`, and reactions `thumbsup` and `white_check_mark` are added to the message in that order.
- **AC-002**: Given a config with one rule and a cursor file containing channel `C0` → ts `T1`, when the binary restarts and the next poll returns messages with ts in `(T1, now]`, then only those messages are evaluated; no command is spawned for messages with ts ≤ `T1`.
- **AC-003**: Given two rules with the same `channel` but different `message` strings, when a message matches both, then both commands are spawned (in order).
- **AC-004**: Given a rule with `regex: true` and `message: "^deploy (prod|staging)$"`, when a message with text `"deploy prod"` arrives, then the command runs; when text is `"deploy dev"` arrives, it does not.
- **AC-005**: Given a missing `--config` flag, the binary exits 1 with a clear error message before opening any Slack connection.
- **AC-006**: Given an invalid regex in a rule, the binary exits 1 on startup and names the offending rule index.
- **AC-007**: Given a triggered command that exits non-zero, the binary logs the exit code, does not crash, and continues processing subsequent messages.
- **AC-008**: Given a triggered command that runs longer than 5 minutes, the binary logs a timeout warning and continues; the command process is left running (v1 does not kill long-running commands — see Section 7).
- **AC-009**: Given a SIGINT, the binary flushes the cursors file and exits 0 within 5 seconds.
- **AC-010**: Given a rule whose `channel` does not exist in the workspace, the binary logs an error on startup listing all unresolved channels and exits 1.
- **AC-011**: Given a matching message, when the binary processes it, then a `thumbsup` reaction is added to that message **before** any matching command is spawned (verifiable via reaction order in logs).
- **AC-012**: Given a triggered command that exits with code `0`, then a `white_check_mark` reaction is added to the originating message.
- **AC-013**: Given a triggered command that exits with any non-zero code, then an `x` reaction is added to the originating message.
- **AC-014**: Given a message authored by the authenticated user (user id matches the token's user id), then the binary adds no reaction and spawns no command for that message.
- **AC-015**: Given the binary has already added `thumbsup` to a message (e.g. after a restart resume where the same message is somehow re-encountered), then a second `thumbsup` add attempt does not fail the run (`already_reacted` is treated as success).
- **AC-016**: Given a `reactions.add` failure other than `already_reacted` (e.g. `missing_scope`, rate limit), then the binary logs the error and continues with command execution.
- **AC-017**: Given a fresh install with no cursor file, the first poll for each channel fetches the most recent 100 messages, seeds the cursor to the newest returned `ts`, and spawns **no** commands.

## 6. Test Automation Strategy

- **Test Levels**: Unit tests for rule evaluation, regex compilation, env-var injection, cursor file rotation, self-message filter. Integration test with a recorded Slack Web API HTTP fixture (JSON) for end-to-end flow.
- **Frameworks**: `cargo test` (built-in), `assert_matches`, `tempfile` for fixture dirs, `wiremock` for HTTP mocks of `slack.com`. No additional test-only dependencies beyond the minimum.
- **Test Data Management**: A `tests/fixtures/` directory with sample rule JSON files, sample `conversations.history` response JSON, and a recorded `reactions.add` flow. Fixtures are checked in.
- **CI/CD Integration**: GitHub Actions matrix: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`. No release job in v1.
- **Coverage Requirements**: None enforced; line coverage should naturally exceed 70% given the small surface area.
- **Performance Testing**: None in v1. The hot path is rule lookup + subprocess spawn; well under Slack's event rate.

## 7. Rationale & Context

### Why polling instead of Socket Mode / Events API

The operator provisions a **user token only** (no app-level token, no Slack app config beyond a one-time scope grant). That rules out Socket Mode (`connections:write` is an app scope) and Events API (requires a public HTTPS endpoint). The remaining option is Web API polling. Trade-offs:

- **Latency** = poll interval. Default 10 s is acceptable for CI / webhook triggers. Sub-second latency is out of scope.
- **Rate limits**: `conversations.history` is tier 3 (~50 req/min). With ≤ 5 watched channels and a 10 s interval, we use ~30 req/min — comfortably under the cap.
- **Polling vs cron**: `listening-to` schedules its poll via cron. `wf-trigger` uses an in-process `tokio::time::interval` loop instead so that SIGINT drains in-flight commands before exiting, which cron cannot do cleanly.

### Why no hot reload

Restart is one second of downtime. The complexity of `notify` + atomic config swap + in-flight command cancellation is not justified for a single-user tool. Marked `ponytail:` for later if needed.

### Why persist cursors per channel

Polling re-fetches on every cycle. Without persisting `latest` `ts` per channel, a restart would either replay up to the most recent 100 messages (firing every command again) or miss messages that arrived during downtime. Persisting the cursor makes restarts resume exactly where the last clean poll left off. The file is tiny (one string per channel) and rewritten atomically.

### Why a no-self-reaction filter

The authenticated user can post messages in the same channels the binary watches. Reacting to one's own messages clutters the thread and produces false-positive `:thumbsup:` notifications. The filter compares the message's `user` field against the `auth.test` response cached at startup.

### Why no command timeout enforcement in v1

`tokio::process::Command::kill` is one line, but the interaction with the cursors file (should we move the cursor past a trigger we then killed?) is the real design question. Default: leave the command running, log a warning. Document the trade-off.

### Why a single cursors file per config

One process, one config, one file. Multi-config or multi-instance deployments are out of scope; users wanting parallelism run multiple instances with separate configs and separate cursors files (the path is derived from the config path).

## 8. Dependencies & External Integrations

### External Systems

- **EXT-001**: Slack workspace — Web API only. `auth.test` once at startup, `conversations.list` / `groups.list` for channel-name resolution, `conversations.history` per watched channel every `--poll-interval` seconds, `reactions.add` per trigger outcome.

### Third-Party Services

- **SVC-001**: Slack — required. Single workspace. Operator installs a custom Slack app (https://api.slack.com/apps) once to grant a user token with the scopes listed in Section 4.

### Infrastructure Dependencies

- **INF-001**: Outbound HTTPS to `slack.com` (api.slack.com) for all Web API calls. Outbound DNS.

### Data Dependencies

- **DAT-001**: Local rule JSON file (read once at startup). Operator-owned.
- **DAT-002**: Local cursors file (read at startup, rewritten after every poll). Operator-owned.

### Technology Platform Dependencies

- **PLT-001**: Rust stable ≥ 1.85 (edition 2024). Linux glibc ≥ 2.31 for the prebuilt x86_64 binary.

### Compliance Dependencies

- None. Single-user tool.

## 9. Examples & Edge Cases

### Minimal config

```json
[
  {
    "channel": "deploys",
    "message": "!deploy prod",
    "command": "curl -fsS -X POST https://ci.example.com/deploy?env=prod"
  }
]
```

### Multi-rule config

```json
[
  {
    "channel": "C0123",
    "message": "build green",
    "command": "notify-send 'CI' 'main is green'"
  },
  {
    "channel": "alerts",
    "message": "high cpu",
    "regex": true,
    "command": "ssh prod 'sudo systemctl restart myapp'"
  },
  {
    "channel": "C0123",
    "message": "release ",
    "command": "/opt/release.sh"
  }
]
```

### Edge cases handled

- Rule mentions a channel the user is not in → startup error (AC-010).
- Rule `regex: true` with invalid regex → startup error (AC-006).
- Command exits 137 (SIGKILL) → logged with exit code; `x` reaction added; no retry.
- Slack API returns `429 Too Many Requests` → poll sleeps the remainder of the window before retrying; cursors are not advanced for the failed channel.
- Same `message` substring appears in a thread reply → each top-level message and each thread reply is its own cursor entry; both may trigger independently.
- Authenticated user posts in a watched channel → ignored (REQ-014).
- `reactions.add` returns `already_reacted` → treated as success, command execution proceeds normally (REQ-014, AC-015).

### Edge cases NOT handled in v1

- The user is removed from a channel mid-run → the next poll for that channel will return `not_in_channel`; logged as error and the cursor is not advanced. Operator must restart with updated config.
- Commands that fork daemons → expected; not tracked.
- Messages edited in Slack → edits do NOT re-trigger. Polling only sees the original `ts`.
- Messages deleted in Slack → deletions are ignored.

## 10. Validation Criteria

A change is spec-compliant when:

- All `REQ-*` are implemented or explicitly deferred in the PR description.
- All `AC-*` pass in `cargo test`.
- `cargo clippy --all-targets -- -D warnings` is clean.
- The cursors file is written atomically (write to `.tmp`, rename) per Section 4.
- `--help` mentions the SEC-001 caveat and lists required Slack scopes.

## 11. Related Specifications / Further Reading

- `listening-to` repository — https://github.com/ilteoood/listening-to — for comparison of Slack API usage patterns and env-var conventions.
- Slack Web API `conversations.history` — https://api.slack.com/methods/conversations.history
- Slack Web API `reactions.add` — https://api.slack.com/methods/reactions.add
- Slack user token scopes — https://api.slack.com/scopes
- `tokio` runtime — https://docs.rs/tokio
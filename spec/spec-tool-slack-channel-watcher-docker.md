---
title: Docker Image — Specification
version: 0.1.1
date_created: 2026-07-05
last_updated: 2026-07-05
owner: ilteoood
tags: [tool, docker, distribution, slack-wf-trigger]
---

# Introduction

This document specifies the OCI image build, image contract, and runtime deployment of `slack-wf-trigger`. It complements the main `spec/spec-tool-slack-channel-watcher.md` (which describes the binary itself) by defining how that binary is packaged, distributed, and run as a container.

The image is the official distribution channel starting with v0.1.0; `cargo install` remains supported but is no longer the recommended path.

## 1. Purpose & Scope

### Purpose

Provide a reproducible OCI image that:

- Compiles the `slack-wf-trigger` binary from source in a multi-stage build with reproducible dependency resolution.
- Ships a minimal runtime image that contains only the binary, CA certificates, and `/bin/sh`.
- Runs as a non-root user.
- Persists the cursors file across container restarts via a Docker volume.
- Publishes to GitHub Container Registry (`ghcr.io/ilteoood/slack-wf-trigger`).

### Scope

**In scope:**

- One architecture in v1: `linux/amd64`.
- Multi-stage `Dockerfile` checked into the repo root.
- `.dockerignore` checked into the repo root.
- Runtime contract: env vars, default config path, default cursors path, user, exposed signal handling.
- Image versioning aligned with Cargo crate version.
- `docker build` and `docker run` examples in the README.

**Out of scope (v1):**

- Multi-arch builds (`linux/arm64`, `linux/arm/v7`) — see `ponytail:` note in Section 7.
- Multi-arch manifests / image index.
- `docker-compose.yml` (operator owns their compose setup).
- Helm chart / Kubernetes manifests.
- Auto-update of the image (watchtower, etc.).
- SBOM / SLSA provenance attestation.
- Cosign signature.
- Release pipeline automation (manual `docker build && docker push` for v1; GH Actions publishes in v2).

### Audience

- `ilteoood` (the operator). Single-user image; no multi-tenant hardening.

## 2. Definitions

| Term | Definition |
|---|---|
| Builder stage | First stage of the multi-stage build. Compiles the Rust binary against a static musl target. |
| Runtime stage | Final stage of the build. Contains only what is needed to run the binary. |
| `musl` target | Rust target triple `x86_64-unknown-linux-musl`. Produces a statically-linked binary that requires no libc at runtime. |
| `nobody` | Linux uid 65534. The image's runtime user. Created via a minimal `/etc/passwd` line injected into the runtime stage. |
| CA bundle | `/etc/ssl/certs/ca-certificates.crt` copied from the builder stage. Required for `reqwest` over `rustls` to validate the Slack Web API TLS chain. |
| Pin | A specific image tag that does not move (e.g. `0.1.0`). Distinct from a floating tag (`latest`). |
| Runtime contract | The set of env vars, volume mount points, default paths, signal semantics, and exit codes the image guarantees. Operators rely on the contract; the image enforces it. |

## 3. Requirements, Constraints & Guidelines

### Functional Requirements

- **REQ-101**: The repo shall contain a `Dockerfile` at the root that builds the `slack-wf-trigger` binary via a multi-stage build.
- **REQ-102**: The repo shall contain a `.dockerignore` at the root that excludes `target/`, `.git/`, `tests/`, `spec/`, `*.md` other than `README.md`, and any local editor / OS metadata.
- **REQ-103**: The builder stage shall compile the binary with the `x86_64-unknown-linux-musl` target, producing a statically linked executable named `slack-wf-trigger`.
- **REQ-104**: The runtime stage shall inherit from `alpine:3.20` (pinned; not `:latest`). The binary is copied from the builder stage with mode `0755`.
- **REQ-105**: The runtime stage shall copy `/etc/ssl/certs/ca-certificates.crt` from the builder stage. The `SSL_CERT_FILE` env var shall be set to its path inside the image so `reqwest` over `rustls` validates Slack's TLS chain.
- **REQ-106**: The runtime stage shall run as uid 65534 (`nobody`). The image shall not require `docker run -u 0`.
- **REQ-107**: The `ENTRYPOINT` shall be the exec form `["/slack-wf-trigger"]`. No shell wrapper. SIGTERM from `docker stop` reaches the binary directly.
- **REQ-108**: The default value of `SLACK_WF_TRIGGER_CONFIG` inside the image shall be `/etc/slack-wf-trigger/rules.json`. The runtime stage shall create `/etc/slack-wf-trigger` and `/var/lib/slack-wf-trigger` directories, owned by `nobody`.
- **REQ-109**: The default value of `SLACK_WF_TRIGGER_POLL_INTERVAL` inside the image shall be `10`. The default value of `RUST_LOG` shall be `info`.
- **REQ-110**: Tag scheme: image tags follow the Cargo crate version verbatim. v0.1.0 → `:0.1.0`. The mutable `:latest` tag tracks the newest release. Pre-release tags (`:0.2.0-rc.1`) are allowed.
- **REQ-111**: The image shall be published to `ghcr.io/ilteoood/slack-wf-trigger`. Auth uses the `GITHUB_TOKEN` in CI; manual publish uses `docker login ghcr.io`.
- **REQ-112**: A `docker build` from a clean checkout shall complete in under 10 minutes on a cold cache on a developer laptop, and in under 90 seconds with warm dependency cache. Achieved by ordering `COPY` so that `Cargo.toml` and `Cargo.lock` are copied before `src/`.
- **REQ-113**: Compressed image size shall be under 50 MB. Achieved by `alpine:3.20` (~5 MB) + static binary (~3 MB stripped) + ca-certs (~0.5 MB) + `/bin/sh` (~0.1 MB).

### Security Requirements

- **SEC-101**: The binary shall not run as root. `USER nobody` is set in the runtime stage.
- **SEC-102**: No secrets shall be baked into the image. `SLACK_USER_TOKEN` has no `ENV` default; the absence of the var makes the binary exit 1 at startup, per the main spec's REQ-010.
- **SEC-103**: The base image shall be pinned by digest in CI builds. Local `docker build` may use the tag for ergonomics; release builds use `alpine:3.20@sha256:<digest>`.
- **SEC-104**: The `:latest` tag shall not be the only pin in deployment. Operators are required to pin a specific version in compose / Kubernetes manifests; documented in the README.
- **SEC-105**: The image shall ship no `curl`, `wget`, `apt`, or other network binaries. The only binaries are `slack-wf-trigger` and `/bin/sh` (from Alpine).

### Constraints

- **CON-101**: Builder base image is `rust:1.85-alpine` (pinned; matches `rust-version` in `Cargo.toml`).
- **CON-102**: Runtime base image is `alpine:3.20` (pinned, not `:latest`).
- **CON-103**: Single architecture for v1: `linux/amd64`. `linux/arm64` deferred (see Section 7).
- **CON-104**: No new toolchain in the repo beyond `docker` (or `docker buildx`) ≥ 24.0. No `cargo-chef`, no `sccache`, no `cross` in v1.
- **CON-105**: The Dockerfile shall use `Dockerfile 1.7` syntax. `# syntax=docker/dockerfile:1.7` is the first line.

### Guidelines

- **GUD-101**: ORDER `COPY` for cache efficiency: `Cargo.toml`, `Cargo.lock` first (cached), `src/` second (invalidates build cache for code changes only).
- **GUD-102**: Use the standard "fake `main.rs` then real `src/`" pattern: an empty `main.rs` is created on the first `RUN` so dependencies are resolved and cached, then real sources are `COPY`ed in for the final compile.
- **GUD-103**: Do not enable `cargo-chef` or `sccache` in v1; the simple two-pass pattern is enough for one source tree of this size.
- **GUD-104**: Document env vars in the README's Docker section, not in the Dockerfile's `ENV` lines. The Dockerfile sets only the truly-constant defaults (`PATH`, `RUST_LOG`, `SSL_CERT_FILE`, `SLACK_WF_TRIGGER_CONFIG`, `SLACK_WF_TRIGGER_POLL_INTERVAL`).
- **GUD-105**: Prefer exec form in `ENTRYPOINT` and `CMD` to avoid a shell wrapper layer and to ensure signals reach the binary.
- **GUD-106**: Use `WORKDIR /build` in the builder, not `/root`, so the build doesn't bake a `root` ownership.

## 4. Interfaces & Data Contracts

### Image Contract

| Aspect | Value | Source |
|---|---|---|
| Image registry | `ghcr.io/ilteoood/slack-wf-trigger` | REQ-111 |
| Tags | `<version>`, `:latest`, `:0.1.0-rc.1` | REQ-110 |
| Entrypoint | `["/slack-wf-trigger"]` | REQ-107 |
| Default user | `nobody` (uid 65534) | REQ-106 |
| Working directory | `/var/lib/slack-wf-trigger` | REQ-108 |
| TLS CA bundle | `/etc/ssl/certs/ca-certificates.crt`, `SSL_CERT_FILE` points to it | REQ-105 |
| Default config path | `/etc/slack-wf-trigger/rules.json` (`SLACK_WF_TRIGGER_CONFIG`) | REQ-108 |
| Default cursors path | `<config-dir>/.slack-wf-trigger.cursors.json` (i.e. `/etc/slack-wf-trigger/.slack-wf-trigger.cursors.json` if not redirected) — see REQ-114 | REQ-114 |
| Default poll interval | `10` seconds (`SLACK_WF_TRIGGER_POLL_INTERVAL`) | REQ-109 |
| Default log filter | `info` (`RUST_LOG`) | REQ-109 |

- **REQ-114**: The cursors file path shall be overridable via `SLACK_WF_TRIGGER_CURSORS_PATH` env var (default `<config-dir>/.slack-wf-trigger.cursors.json`). If unset, the binary writes the cursors file next to the config. Operators who want the cursors file outside `/etc/slack-wf-trigger` should mount `/var/lib/slack-wf-trigger` and override `SLACK_WF_TRIGGER_CURSORS_PATH` to e.g. `/var/lib/slack-wf-trigger/.slack-wf-trigger.cursors.json`. This requires the main spec's REQ-008 to be relaxed to honor the env var; the relaxation is in scope for v1 and called out as a forward-compatibility item in Section 7.

### Environment Variables (Image-Documented)

| Var | Purpose | Required | Default in image |
|---|---|---|---|
| `SLACK_USER_TOKEN` | Slack user token (`xoxp-...`). Per main spec SEC-002. | Yes | unset — startup fails if missing. |
| `SLACK_WF_TRIGGER_CONFIG` | Path to rules JSON. | Yes (default provided) | `/etc/slack-wf-trigger/rules.json` |
| `SLACK_WF_TRIGGER_POLL_INTERVAL` | Poll interval in seconds. | No | `10` |
| `SLACK_WF_TRIGGER_CURSORS_PATH` | Override the cursors file path. | No | `<config-dir>/.slack-wf-trigger.cursors.json` |
| `RUST_LOG` | Tracing-subscriber filter. | No | `info` |

### Volumes

| Volume mount point | Purpose | Source control |
|---|---|---|
| `/etc/slack-wf-trigger` | Rules JSON. Read-only is OK in steady state; binary does not write here. Bind-mount or named volume. | Operator |
| `/var/lib/slack-wf-trigger` | Recommended cursors file location when `SLACK_WF_TRIGGER_CURSORS_PATH` is overridden. | Operator |

If the operator uses the default cursors location (no `SLACK_WF_TRIGGER_CURSORS_PATH` override), the cursors file is written to `/etc/slack-wf-trigger/.slack-wf-trigger.cursors.json`. The binary does not require `/etc/slack-wf-trigger` to be writable unless the operator uses the default. Recommended setup: override `SLACK_WF_TRIGGER_CURSORS_PATH=/var/lib/slack-wf-trigger/.slack-wf-trigger.cursors.json` and mount `/var/lib/slack-wf-trigger` as a named volume.

### Signals

| Signal | Behavior |
|---|---|
| `SIGTERM` (from `docker stop`) | Reaches the binary directly (exec form `ENTRYPOINT`). The binary flushes the cursors file and exits 0 within 5 s, per the main spec's AC-009. |
| `SIGINT` (Ctrl-C in foreground) | Same as SIGTERM. |
| `SIGKILL` | Not handled. `docker stop --time=10` will escalate after the grace period. |

### Image Layers (Builder Stage)

| Step | Purpose | Cache key |
|---|---|---|
| `FROM rust:1.85-alpine` | Base with stable Rust | `rust:1.85-alpine` digest (implicit) |
| `RUN apk add --no-cache musl-dev` | C runtime headers for the musl target | `apk add` layer; stable |
| `WORKDIR /build` | Avoid baking `/root` ownership | layer metadata |
| `COPY Cargo.toml Cargo.lock ./` | Dependency manifest | invalidated on Cargo.toml change |
| `RUN mkdir -p src && echo 'fn main(){}' > src/main.rs && cargo build --release && rm -rf src` | Resolve & cache dependencies | invalidated on Cargo.toml change; reused across source-only changes |
| `COPY src ./src` | Real sources | invalidated on any source change |
| `RUN touch src/main.rs && cargo build --release` | Final compile | rebuilt on source change |
| `RUN cp target/release/slack-wf-trigger /slack-wf-trigger && strip /slack-wf-trigger` | Strip + relocate | rebuilt on final compile |

### Image Layers (Runtime Stage)

| Step | Purpose |
|---|---|
| `FROM alpine:3.20` | Minimal base with `/bin/sh` (required per REQ-006 in the main spec). |
| `COPY --from=builder /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/ca-certificates.crt` | CA bundle for `reqwest` over `rustls`. |
| `COPY --from=builder /slack-wf-trigger /slack-wf-trigger` | The binary. |
| `RUN addgroup -g 65534 nobody && adduser -u 65534 -G nobody -D -H nobody` | Create `nobody` user inside Alpine. (Alternative: copy a minimal `/etc/passwd` line from builder as in `listening-to`. Either is acceptable; this version is documented here.) |
| `RUN mkdir -p /etc/slack-wf-trigger /var/lib/slack-wf-trigger && chown -R nobody:nobody /var/lib/slack-wf-trigger /etc/slack-wf-trigger` | Mount points. |
| `USER nobody` | Drop root. |
| `WORKDIR /var/lib/slack-wf-trigger` | Default cwd. |
| `ENV PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin` | Sane PATH so `/bin/sh` is reachable. |
| `ENV SSL_CERT_FILE=/etc/ssl/certs/ca-certificates.crt RUST_LOG=info SLACK_WF_TRIGGER_CONFIG=/etc/slack-wf-trigger/rules.json SLACK_WF_TRIGGER_POLL_INTERVAL=10` | Defaults per REQ-108, REQ-109, REQ-105. |
| `ENTRYPOINT ["/slack-wf-trigger"]` | Exec form. |

## 5. Acceptance Criteria

- **AC-101**: `docker build -t slack-wf-trigger:dev .` from a clean checkout produces a runnable image in under 10 minutes cold, under 90 s warm.
- **AC-102**: `docker inspect --format='{{.Config.User}}' slack-wf-trigger:dev` prints `nobody` (or `65534`).
- **AC-103**: `docker inspect --format='{{.Config.Entrypoint}}' slack-wf-trigger:dev` prints `[/slack-wf-trigger]`.
- **AC-104**: `docker image ls slack-wf-trigger:dev` reports a size under 50 MB.
- **AC-105**: Given a rules JSON mounted at `/etc/slack-wf-trigger/rules.json` and `SLACK_USER_TOKEN` set, `docker run --rm slack-wf-trigger:dev` connects to Slack and emits the expected startup logs within 5 s. If `SLACK_USER_TOKEN` is unset, the process exits non-zero within 5 s with a clear error message (per main spec REQ-010).
- **AC-106**: A second `docker run` of the same image with the same cursors file mounted resumes from the persisted cursor and does not re-trigger commands for messages already processed (per main spec AC-002).
- **AC-107**: `docker stop` of a running container exits with code 0 within 10 s of SIGTERM. The cursors file on the mounted volume is flushed before exit (per main spec AC-009).
- **AC-108**: Triggered commands running inside the container can invoke `/bin/sh -c '...'` (per main spec REQ-006). Verifiable by adding a rule whose `command` is `echo $$` and observing the container's PID via the logs.
- **AC-109**: A request to `https://slack.com/api/auth.test` from inside the running container succeeds (verifies the CA bundle and `SSL_CERT_FILE` are correctly wired). Test: `docker exec <c> sh -c 'wget -qO- "$SLACK_USER_TOKEN" >/dev/null'` is NOT a valid test (no wget). Valid test: run any rule that calls Slack and confirm the Web API call succeeds.
- **AC-110**: `docker scan slack-wf-trigger:dev` (or `trivy image`) reports no critical CVEs from the Alpine base layer beyond the standard Alpine CVE cycle, and zero CVEs attributable to `slack-wf-trigger`'s own dependencies.

## 6. Test Automation Strategy

- **Local sanity**: `docker build -t slack-wf-trigger:dev .` after every change to `Cargo.toml`, `Cargo.lock`, or `Dockerfile`. Time the build.
- **Smoke test (manual, one-shot)**: `docker run --rm -e SLACK_USER_TOKEN=$TOKEN -v $(pwd)/example-rules.json:/etc/slack-wf-trigger/rules.json slack-wf-trigger:dev --help` should print help and exit 0 (binary reads `--help` flag without touching Slack).
- **Integration test (manual)**: a rule whose `command` is `echo $SLACK_WF_TRIGGER_TS > /tmp/last_ts` proves that env-var injection still works inside the container.
- **CI (planned v2)**: GitHub Actions job that builds the image on PR, then runs `trivy image` and `docker run --rm <image> --help` as required checks.
- **Release publish (planned v2)**: GitHub Actions release job triggered by a `v*.*.*` tag, builds the image, pushes to `ghcr.io/ilteoood/slack-wf-trigger:<version>` and `:latest`. v1 publishing is manual.

## 7. Rationale & Context

### Why Alpine (not `scratch`) as the runtime base

The main spec's REQ-006 mandates `sh -c` for triggered commands. `scratch` ships no shell. The closest alternative is to copy `/bin/sh` from Alpine into a `scratch` image together with its dynamic linker and dependencies, but that is fragile across Alpine releases and offers negligible size benefit (a few hundred KB). Alpine final gives us `/bin/sh` for ~5 MB total, which is well under REQ-113's 50 MB budget. `listening-to` uses `scratch` because it spawns no user-supplied commands — its `sh -c` chain is internal to the tool. The two tools diverge here and that's fine. Flagged as a deviation from the `listening-to` pattern.

### Why multi-stage

The binary must be statically linked with musl and stripped. Building on Alpine and copying the resulting binary into a slim runtime stage keeps the build environment out of the runtime image. Standard Rust-on-Alpine pattern.

### Why `alpine:3.20` pin (not `:latest`)

Reproducibility. `:latest` is a moving target; a re-pull six months later could change the contents of `/etc/ssl/certs/ca-certificates.crt` and (rarely) `/bin/sh`'s behavior. Pinning the minor version freezes those. CI additionally pins by digest per SEC-103.

### Why exec form for `ENTRYPOINT`

Shell-form `ENTRYPOINT` (e.g. `ENTRYPOINT slack-wf-trigger`) wraps the binary in `/bin/sh -c`, which means `docker stop` sends SIGTERM to the shell. The shell then forwards to the binary — or doesn't, depending on the shell. Exec form drops this footgun: signals reach the binary directly.

### Why a forward-compat `SLACK_WF_TRIGGER_CURSORS_PATH`

Putting the cursors file alongside the config file is awkward once the config is bind-mounted read-only. The main spec currently writes cursors next to config (REQ-008). The Docker image exposes this knob so operators can split config and state without patching the binary. The relaxation of REQ-008 (honor `SLACK_WF_TRIGGER_CURSORS_PATH` if set) is in scope for this spec and is the only cross-spec change required.

### Why no multi-arch in v1

`ponytail:` Cross-compiling Rust to `aarch64-unknown-linux-musl` requires the same Alpine chain on ARM, or `cross` + QEMU. The tool runs only on one machine today (`linux/amd64`). Adding `linux/arm64` doubles the build matrix and CI time for zero current users. Add when a Raspberry Pi enters the fleet; the builder change is `rustup target add aarch64-unknown-linux-musl` + a second `cargo build --release --target=...` + a multi-arch `FROM` in the runtime stage.

### Why no `cargo-chef` in v1

`cargo-chef` is a 30-line diff in the Dockerfile for what `Cargo.toml` dependency-cache invalidation already gives us. The two-pass "fake `main.rs`" trick is functionally equivalent for a project this size. Re-evaluate if a CI rebuild ever exceeds the 90 s warm-budget from REQ-112.

### Why no `SLACK_USER_TOKEN` default

Secrets in image layers are world-readable to anyone who can `docker pull`. The main spec already requires the var at runtime (REQ-010 / SEC-002); the image respects that, with no default and no warning suppression.

## 8. Dependencies & External Integrations

### External Systems

- **EXT-101**: GitHub Container Registry at `ghcr.io`. Auth via `GITHUB_TOKEN` (CI) or `docker login ghcr.io` (local).
- **EXT-102**: Docker Hub is not used in v1.

### Third-Party Services

- None.

### Infrastructure Dependencies

- **INF-101**: Docker Engine ≥ 24.0 (for BuildKit and `Dockerfile 1.7` syntax).
- **INF-102**: Outbound HTTPS from the Docker host to `slack.com` (the same requirement as the main spec's INF-001). Mounted config and cursors volumes are local-only.

### Data Dependencies

- **DAT-101**: `Cargo.toml` and `Cargo.lock` at the repo root. Read at builder stage.
- **DAT-102**: `src/` directory. Read at builder stage.
- **DAT-103**: Rules JSON at `/etc/slack-wf-trigger/rules.json` (or wherever `SLACK_WF_TRIGGER_CONFIG` points). Read at runtime.
- **DAT-104**: Cursors JSON at the configured path. Read and written at runtime.

### Technology Platform Dependencies

- **PLT-101**: Rust stable ≥ 1.85 (provided by `rust:1.85-alpine`).
- **PLT-102**: Alpine Linux 3.20 musl libc (provided by `alpine:3.20`).

### Compliance Dependencies

- None.

## 9. Examples & Edge Cases

### Build

```bash
docker build -t slack-wf-trigger:dev .
docker build -t ghcr.io/ilteoood/slack-wf-trigger:0.1.0 .
```

### Run (smoke test)

```bash
docker run --rm slack-wf-trigger:dev --help
```

### Run (real)

```bash
docker run -d \
  --name slack-wf-trigger \
  --restart unless-stopped \
  -e SLACK_USER_TOKEN=xoxp-... \
  -e RUST_LOG=info,slack_wf_trigger=debug \
  -v $PWD/rules.json:/etc/slack-wf-trigger/rules.json:ro \
  -v slack-wf-trigger-state:/var/lib/slack-wf-trigger \
  -e SLACK_WF_TRIGGER_CURSORS_PATH=/var/lib/slack-wf-trigger/.slack-wf-trigger.cursors.json \
  ghcr.io/ilteoood/slack-wf-trigger:0.1.0
```

### Edge cases handled

- `docker run` without `SLACK_USER_TOKEN` → the binary exits non-zero within 5 s with the message required by REQ-010. The container stops; no infinite retry loop. (Operators add `restart: on-failure: 3` if they want bounded retries.)
- Config file is missing inside the container → the binary exits non-zero. `docker run` exit code propagates. No silent polling loop.
- Volume not mounted for cursors file → cursors are written into the container's writable layer. On `docker rm` they're lost. Documented; operator's job to mount a volume.
- `docker stop` during an in-flight triggered command → the binary's signal handler flushes cursors but does NOT kill the spawned subprocess (per main spec's "no command timeout enforcement in v1"). The child may continue running until the container's PID 1 exits; if the kernel reaps the child, it gets SIGKILL on container exit.
- Image pulled with a stale digest → TLS validation fails (`reqwest` rejects the cert); the binary logs a connection error and the operator notices.

### Edge cases NOT handled in v1

- `linux/arm64` — see Section 7.
- Auto-restart on transient Slack 5xx — restart policy is the operator's call (`--restart on-failure:5` is a reasonable default but is not baked into the image).
- Config hot reload — main spec already defers this; the image doesn't reopen it.
- Resource limits (`--memory`, `--cpus`) — operator's call.

## 10. Validation Criteria

A change to the Dockerfile, `.dockerignore`, or anything in `src/` or `Cargo.toml` is image-spec-compliant when:

- `docker build -t slack-wf-trigger:dev .` succeeds locally.
- `docker run --rm slack-wf-trigger:dev --help` exits 0.
- Compressed image size is under 50 MB (`docker image ls --format='{{.Size}}'` shows the uncompressed size; divide by ~3 for compressed).
- `USER` is `nobody` (AC-102).
- `ENTRYPOINT` is `[/slack-wf-trigger]` exec form (AC-103).
- `SSL_CERT_FILE` is set to a path inside the image that exists at runtime (AC-109).
- Any change to `Cargo.toml` without a corresponding `Cargo.lock` update fails CI (planned v2; manual check in v1).

## 11. Related Specifications / Further Reading

- `spec/spec-tool-slack-channel-watcher.md` — main spec; this document is its packaging counterpart.
- `listening-to` repository — https://github.com/ilteoood/listening-to — reference pattern for the multi-stage build and `nobody` user; `slack-wf-trigger` diverges from this pattern by using Alpine (not `scratch`) for the runtime base (see Section 7).
- Alpine 3.20 release notes — https://alpinelinux.org/posts/Alpine-3.20.0-released/
- Docker `Dockerfile` reference — https://docs.docker.com/reference/dockerfile/
- `docker stop` semantics — https://docs.docker.com/reference/cli/docker/container/stop/
- GitHub Container Registry — https://docs.github.com/en/packages/working-with-a-github-packages-registry/working-with-the-container-registry
- Rust musl target — https://doc.rust-lang.org/nightly/rustc/platform-support/x86_64-unknown-linux-musl.html
- `reqwest` rustls feature — https://docs.rs/reqwest

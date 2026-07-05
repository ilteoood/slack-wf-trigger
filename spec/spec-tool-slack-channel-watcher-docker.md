---
title: Docker Image — Specification
version: 0.4.0
date_created: 2026-07-05
last_updated: 2026-07-05
owner: ilteoood
tags: [tool, docker, distribution, slack-wf-trigger, multi-arch, compose, ci]
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

- Two architectures: `linux/amd64` and `linux/arm64`.
- Multi-stage `Dockerfile` checked into the repo root.
- `.dockerignore` checked into the repo root.
- `docker-compose.yml` checked into the repo root as the recommended local-deployment entry point.
- Runtime contract: env vars, default config path, default cursors path, user, exposed signal handling.
- Image versioning aligned with Cargo crate version.
- Multi-arch manifest (image index) published to the registry.
- GitHub Actions workflows for CI (PR builds + lint + test + scan) and release (`v*.*.*` tag publish to `ghcr.io`).
- `docker buildx build`, `docker compose`, and `docker run` examples in the README.

**Out of scope (v1):**

- Other architectures (`linux/arm/v7`, `linux/ppc64le`, `linux/s390x`).
- Helm chart / Kubernetes manifests.
- Auto-update of the image (watchtower, etc.).
- SBOM / SLSA provenance attestation.
- Cosign signature.
- Compose overrides for dev (hot-reload, source mounts, etc.).
- Cross-repo release automation (dependabot, release-please).

### Audience

- `ilteoood` (the operator). Single-user image; no multi-tenant hardening.

## 2. Definitions

| Term | Definition |
|---|---|
| Builder stage | First stage of the multi-stage build. Compiles the Rust binary against a static musl target for the target architecture. Runs natively via `BUILDPLATFORM` so QEMU is not invoked during compilation. |
| Runtime stage | Final stage of the build. Inherits `TARGETPLATFORM` so the runtime base image matches the target arch. Contains only what is needed to run the binary. |
| `musl` target | Rust target triple. BuildKit's `TARGETARCH` is mapped to the corresponding Rust triple inside the Dockerfile: `amd64` → `x86_64-unknown-linux-musl`, `arm64` → `aarch64-unknown-linux-musl`. The mapping is computed once and written to `/tmp/rt.env`; subsequent `RUN` steps `. /tmp/rt.env` to read `RUST_TARGET`. Produces a statically-linked binary that requires no libc at runtime. |
| `BUILDPLATFORM` | BuildKit-provided automatic build arg set to the architecture of the builder host. Used on the `FROM` line of the builder stage so native compilation tools run natively; only the Rust compiler emits cross-compiled artifacts. |
| `TARGETPLATFORM` / `TARGETARCH` | BuildKit-provided automatic build args set to the architecture the produced image is for. The musl target triple is derived from `TARGETARCH`. |
| `nobody` | Linux uid 65534. The image's runtime user. Created via a minimal `/etc/passwd` line injected into the runtime stage. |
| CA bundle | `/etc/ssl/certs/ca-certificates.crt` copied from the builder stage. Required for `reqwest` over `rustls` to validate the Slack Web API TLS chain. |
| Pin | A specific image tag that does not move (e.g. `0.1.0`). Distinct from a floating tag (`latest`). |
| Image index | An OCI manifest list (a.k.a. multi-arch manifest) that maps a single tag to one image per supported architecture. Pushed by `docker buildx build --platform ... --push`. |
| Runtime contract | The set of env vars, volume mount points, default paths, signal semantics, and exit codes the image guarantees. Operators rely on the contract; the image enforces it. |

## 3. Requirements, Constraints & Guidelines

### Functional Requirements

- **REQ-101**: The repo shall contain a `Dockerfile` at the root that builds the `slack-wf-trigger` binary via a multi-stage build.
- **REQ-102**: The repo shall contain a `.dockerignore` at the root that excludes `target/`, `.git/`, `tests/`, `spec/`, `*.md` other than `README.md`, and any local editor / OS metadata.
- **REQ-103**: The builder stage shall compile the binary with the musl target for the requested architecture (`${TARGETARCH}-unknown-linux-musl`, where `TARGETARCH` is `amd64` or `arm64`), producing a statically linked executable named `slack-wf-trigger`. The builder stage's `FROM` line shall pin `--platform=$BUILDPLATFORM` so compilation runs natively on the builder host.
- **REQ-104**: The runtime stage shall inherit from `alpine:3.20` (pinned; not `:latest`) with `--platform=$TARGETPLATFORM`. The binary is copied from the builder stage with mode `0755`.
- **REQ-105**: The runtime stage shall copy `/etc/ssl/certs/ca-certificates.crt` from the builder stage. The `SSL_CERT_FILE` env var shall be set to its path inside the image so `reqwest` over `rustls` validates Slack's TLS chain.
- **REQ-106**: The runtime stage shall run as uid 65534 (`nobody`). The image shall not require `docker run -u 0`.
- **REQ-107**: The `ENTRYPOINT` shall be the exec form `["/slack-wf-trigger"]`. No shell wrapper. SIGTERM from `docker stop` reaches the binary directly.
- **REQ-108**: The default value of `SLACK_WF_TRIGGER_CONFIG` inside the image shall be `/etc/slack-wf-trigger/rules.json`. The runtime stage shall create `/etc/slack-wf-trigger` and `/var/lib/slack-wf-trigger` directories, owned by `nobody`.
- **REQ-109**: The default value of `SLACK_WF_TRIGGER_POLL_INTERVAL` inside the image shall be `10`. The default value of `RUST_LOG` shall be `info`.
- **REQ-110**: Tag scheme: image tags follow the Cargo crate version verbatim. v0.1.0 → `:0.1.0`. The mutable `:latest` tag tracks the newest release. Pre-release tags (`:0.2.0-rc.1`) are allowed. Each tag resolves to an OCI image index listing one image per supported architecture.
- **REQ-111**: The image shall be published to `ghcr.io/ilteoood/slack-wf-trigger` as a multi-arch image index. Auth uses the `GITHUB_TOKEN` in CI; manual publish uses `docker login ghcr.io`. Publish command is `docker buildx build --platform linux/amd64,linux/arm64 --push -t ghcr.io/ilteoood/slack-wf-trigger:<tag> .`
- **REQ-111a**: A `docker-compose.yml` shall live at the repo root and expose a single service (`slack-wf-trigger`) that pulls `image: ghcr.io/ilteoood/slack-wf-trigger:${SLACK_WF_TRIGGER_IMAGE_TAG:-0.3.0}`, wires the recommended volume + env layout, and runs as a non-root user (the image's `USER nobody`).
- **REQ-111b**: The compose file shall mount `rules.json` from the host (default `./rules.json`) read-only into `/etc/slack-wf-trigger/rules.json`, and shall mount a named volume `slack-wf-trigger-state` at `/var/lib/slack-wf-trigger` to persist the cursors file. `SLACK_WF_TRIGGER_CURSORS_PATH` shall be set to `/var/lib/slack-wf-trigger/.slack-wf-trigger.cursors.json` so config and state live on different filesystems.
- **REQ-111c**: The compose file shall declare `restart: unless-stopped`, `read_only: false` on the state volume mount only, and shall not require the operator to write any `secrets:` block; `SLACK_USER_TOKEN` is read from the operator's environment via `${SLACK_USER_TOKEN}` interpolation.
- **REQ-111d**: The compose file shall be syntactically valid for both Compose Spec (`docker compose config`) and legacy Compose v2 (`docker-compose config`); verified by `docker compose config -q` returning exit 0.
- **REQ-115**: A CI workflow (`.github/workflows/ci.yml`) shall run on every `push` to `main` and on every PR against `main`. The workflow shall: check out the repo, set up BuildKit-compatible Docker, run `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`, and run `docker buildx build --platform linux/amd64,linux/arm64 --load` followed by `trivy image` against each per-arch image. All checks must pass before the PR can be merged.
- **REQ-116**: A release workflow (`.github/workflows/release.yml`) shall trigger on every `v*.*.*` tag push. It shall log in to `ghcr.io` using the workflow's `GITHUB_TOKEN`, derive `VERSION` from the tag (stripping the leading `v`), run `docker buildx build --platform linux/amd64,linux/arm64 --push` with tags `ghcr.io/ilteoood/slack-wf-trigger:${VERSION}` and `ghcr.io/ilteoood/slack-wf-trigger:latest`, and emit a GitHub Release with the image digest in the notes. Prerelease tags matching `v*.*.*-*` (e.g. `v0.3.0-rc.1`) shall push only the versioned tag and shall NOT update `:latest`; the matching GitHub Release shall be marked as a prerelease.
- **REQ-117**: Both workflows shall pin third-party GitHub Actions by full-length commit SHA (not floating tags). The buildx action, setup-buildx-action, setup-qemu-action, and login-action versions shall be recorded in `requirements.yml` or equivalent pinned references inside the workflow files.
- **REQ-118**: The release workflow shall fail loudly if the Cargo crate version in `Cargo.toml` does not equal the pushed image tag. Drift is detected by reading `Cargo.toml` via `yq` (or grep) and comparing to `${VERSION}`; mismatch exits non-zero.
- **REQ-112**: A `docker buildx build` from a clean checkout shall complete in under 10 minutes on a cold cache on a developer laptop, and in under 90 seconds with warm dependency cache. Achieved by ordering `COPY` so that `Cargo.toml` and `Cargo.lock` are copied before `src/`, and by pinning `--platform=$BUILDPLATFORM` on the builder so cross-compilation skips QEMU.
- **REQ-113**: Compressed image size shall be under 50 MB per-arch. Achieved by `alpine:3.20` (~5 MB) + static binary (~3 MB stripped) + ca-certs (~0.5 MB) + `/bin/sh` (~0.1 MB). The image index adds negligible overhead beyond the sum of its per-arch images.

### Security Requirements

- **SEC-101**: The binary shall not run as root. `USER nobody` is set in the runtime stage.
- **SEC-102**: No secrets shall be baked into the image. `SLACK_USER_TOKEN` has no `ENV` default; the absence of the var makes the binary exit 1 at startup, per the main spec's REQ-010.
- **SEC-103**: The base image shall be pinned by digest in CI builds. Local `docker build` may use the tag for ergonomics; release builds use `alpine:3.20@sha256:<digest>`.
- **SEC-104**: The `:latest` tag shall not be the only pin in deployment. Operators are required to pin a specific version in compose / Kubernetes manifests; documented in the README.
- **SEC-105**: The image shall ship no `curl`, `wget`, `apt`, or other network binaries. The only binaries are `slack-wf-trigger` and `/bin/sh` (from Alpine).

### Constraints

- **CON-101**: Builder base image is `rust:1.96.1-alpine3.24` (pinned; matches `rust-version` in `Cargo.toml`), pinned to `--platform=$BUILDPLATFORM`.
- **CON-102**: Runtime base image is `alpine:3.20` (pinned, not `:latest`), pinned to `--platform=$TARGETPLATFORM`.
- **CON-103**: Supported architectures: `linux/amd64` and `linux/arm64`. Other architectures are out of scope (see Section 7).
- **CON-104**: No new toolchain in the repo beyond `docker` (or `docker buildx`) ≥ 24.0. No `cargo-chef`, no `sccache`, no `cross` in v1.
- **CON-105**: The Dockerfile shall use `Dockerfile 1.7` syntax. `# syntax=docker/dockerfile:1.7` is the first line.
- **CON-106**: CI runners shall be GitHub-hosted `ubuntu-24.04` (provides Docker ≥ 24.0 and BuildKit). Self-hosted runners are out of scope.
- **CON-107**: CI and release workflows shall authenticate to `ghcr.io` exclusively via the workflow's `GITHUB_TOKEN`. No long-lived PATs shall be committed or required as secrets.

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
| Platforms | `linux/amd64`, `linux/arm64` (via OCI image index) | REQ-103, REQ-104, REQ-111 |
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
| Compose state volume | Named volume `slack-wf-trigger-state` mounted at `/var/lib/slack-wf-trigger`. Created by `docker compose up` on first run; persists across `down`/`up` cycles. | REQ-111b |

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
| `FROM --platform=$BUILDPLATFORM rust:1.96.1-alpine3.24 AS builder` | Base with stable Rust, running natively on the builder host | `rust:1.96.1-alpine3.24` digest (implicit) |
| `RUN case "${TARGETARCH}" in amd64) ... arm64) ... esac && rustup target add ...` | Map BuildKit's `TARGETARCH` to the Rust musl target triple and add it to the toolchain. Writes the resolved triple to `/tmp/rt.env`. | stable per arch; rebuilt when `TARGETARCH` changes |
| `ARG TARGETARCH` | Lift BuildKit's auto-arg so the rest of the stage can reference it | implicit; stable per build |
| `RUN rustup target add ${TARGETARCH}-unknown-linux-musl` | Install the musl stdlib for the requested target arch | invalidated when `TARGETARCH` changes |
| `RUN apk add --no-cache musl-dev` | C runtime headers for the musl target | `apk add` layer; stable |
| `WORKDIR /build` | Avoid baking `/root` ownership | layer metadata |
| `COPY Cargo.toml Cargo.lock ./` | Dependency manifest | invalidated on Cargo.toml change |
| `RUN mkdir -p src && echo 'fn main(){}' > src/main.rs && cargo build --release --target ${TARGETARCH}-unknown-linux-musl && rm -rf src target/${TARGETARCH}-unknown-linux-musl/release/deps/slack_wf_trigger*` | Resolve & cache dependencies; remove the fake-main artifact so the next compile rebuilds it | invalidated on Cargo.toml change; reused across source-only changes and across archs (Cargo target dir is keyed per arch) |
| `COPY src ./src` | Real sources | invalidated on any source change |
| `RUN touch src/main.rs && cargo build --release --target ${TARGETARCH}-unknown-linux-musl` | Final compile | rebuilt on source change |
| `RUN cp target/${TARGETARCH}-unknown-linux-musl/release/slack-wf-trigger /slack-wf-trigger && strip /slack-wf-trigger` | Strip + relocate | rebuilt on final compile |

### Image Layers (Runtime Stage)

| Step | Purpose |
|---|---|
| `FROM --platform=$TARGETPLATFORM alpine:3.20` | Minimal base with `/bin/sh` (required per REQ-006 in the main spec), pinned to the target arch. |
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

- **AC-101**: `docker buildx build --platform linux/amd64,linux/arm64 -t slack-wf-trigger:dev .` from a clean checkout produces a runnable image index in under 10 minutes cold, under 90 s warm.
- **AC-102**: `docker inspect --format='{{.Config.User}}' slack-wf-trigger:dev` prints `nobody` (or `65534`) for every per-arch image in the index.
- **AC-103**: `docker inspect --format='{{.Config.Entrypoint}}' slack-wf-trigger:dev` prints `[/slack-wf-trigger]` for every per-arch image in the index.
- **AC-104**: `docker image ls slack-wf-trigger:dev` reports a size under 50 MB per arch.
- **AC-104a**: `docker buildx imagetools inspect slack-wf-trigger:dev` lists both `linux/amd64` and `linux/arm64` in the manifest list.
- **AC-104b**: `docker run --rm --platform linux/arm64 slack-wf-trigger:dev --help` exits 0 on an `amd64` host (cross-arch run via QEMU at runtime only), and the same command without `--platform` on an `arm64` host exits 0 natively.
- **AC-105**: Given a rules JSON mounted at `/etc/slack-wf-trigger/rules.json` and `SLACK_USER_TOKEN` set, `docker run --rm slack-wf-trigger:dev` connects to Slack and emits the expected startup logs within 5 s on either architecture. If `SLACK_USER_TOKEN` is unset, the process exits non-zero within 5 s with a clear error message (per main spec REQ-010).
- **AC-106**: A second `docker run` of the same image with the same cursors file mounted resumes from the persisted cursor and does not re-trigger commands for messages already processed (per main spec AC-002).
- **AC-107**: `docker stop` of a running container exits with code 0 within 10 s of SIGTERM. The cursors file on the mounted volume is flushed before exit (per main spec AC-009).
- **AC-108**: Triggered commands running inside the container can invoke `/bin/sh -c '...'` (per main spec REQ-006). Verifiable by adding a rule whose `command` is `echo $$` and observing the container's PID via the logs.
- **AC-109**: A request to `https://slack.com/api/auth.test` from inside the running container succeeds on both architectures (verifies the CA bundle and `SSL_CERT_FILE` are correctly wired). Test: `docker exec <c> sh -c 'wget -qO- "$SLACK_USER_TOKEN" >/dev/null'` is NOT a valid test (no wget). Valid test: run any rule that calls Slack and confirm the Web API call succeeds.
- **AC-110**: `docker scan slack-wf-trigger:dev` (or `trivy image`) on either per-arch image reports no critical CVEs from the Alpine base layer beyond the standard Alpine CVE cycle, and zero CVEs attributable to `slack-wf-trigger`'s own dependencies.
- **AC-111**: `docker compose config -q` exits 0 against the shipped `docker-compose.yml` and reports a single service `slack-wf-trigger` with the image reference `ghcr.io/ilteoood/slack-wf-trigger:${SLACK_WF_TRIGGER_IMAGE_TAG:-0.3.0}`, a read-only `./rules.json:/etc/slack-wf-trigger/rules.json:ro` mount, a `slack-wf-trigger-state:/var/lib/slack-wf-trigger` named volume, and `SLACK_WF_TRIGGER_CURSORS_PATH=/var/lib/slack-wf-trigger/.slack-wf-trigger.cursors.json` in the environment.
- **AC-112**: `docker compose up -d` followed by `docker compose exec slack-wf-trigger --help` exits 0 on the host's native arch and the cursors volume `slack-wf-trigger-state` is created and listed by `docker volume ls`.
- **AC-113**: Pushing a commit to `feature/*` opens a PR; the CI workflow's `ci / build` job reports the multi-arch build as `success`, and the `ci / lint` job reports `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, and `cargo test` as `success`. A bad PR (e.g. `cargo fmt` drift) is blocked.
- **AC-114**: Pushing the tag `v0.3.0` produces: (a) an image `ghcr.io/ilteoood/slack-wf-trigger:0.3.0` listed by `docker buildx imagetools inspect` with both `linux/amd64` and `linux/arm64`; (b) an image `ghcr.io/ilteoood/slack-wf-trigger:latest` pointing to the same manifest list; (c) a GitHub Release named `0.3.0` (not marked prerelease) whose body contains the multi-arch image index digest.
- **AC-115**: Pushing the tag `v0.4.0-rc.1` produces an image `ghcr.io/ilteoood/slack-wf-trigger:0.4.0-rc.1` but does NOT update `:latest`. The GitHub Release is marked as a prerelease.
- **AC-116**: Pushing a tag whose version does not match the Cargo crate version in `Cargo.toml` fails the release workflow before any image is pushed (per REQ-118).

## 6. Test Automation Strategy

- **Local sanity**: `docker buildx build --platform linux/amd64,linux/arm64 -t slack-wf-trigger:dev .` after every change to `Cargo.toml`, `Cargo.lock`, or `Dockerfile`. Time the build. On an `amd64`-only dev machine, `docker buildx build --platform linux/amd64 -t slack-wf-trigger:dev .` is acceptable for fast iteration; the full matrix runs in CI.
- **Smoke test (manual, one-shot)**: `docker run --rm -e SLACK_USER_TOKEN=$TOKEN -v $(pwd)/example-rules.json:/etc/slack-wf-trigger/rules.json slack-wf-trigger:dev --help` should print help and exit 0 (binary reads `--help` flag without touching Slack). Repeat with `--platform linux/arm64` to exercise the arm64 image.
- **Integration test (manual)**: a rule whose `command` is `echo $SLACK_WF_TRIGGER_TS > /tmp/last_ts` proves that env-var injection still works inside the container.
- **CI (`.github/workflows/ci.yml`)**: runs on `push` to `main` and every PR against `main`. Three required jobs: `lint` (`cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`), `test` (`cargo test`), and `build` (`docker buildx build --platform linux/amd64,linux/arm64 --load` + `trivy image` per-arch). Per REQ-115.
- **Release publish (`.github/workflows/release.yml`)**: triggers on `v*.*.*` tag push. Verifies Cargo crate version matches tag (REQ-118), then `docker buildx build --platform linux/amd64,linux/arm64 --push` to `ghcr.io/ilteoood/slack-wf-trigger:${VERSION}` and `:latest` (non-prerelease tags only), then creates a GitHub Release with the image digest. Per REQ-116.

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

### Why multi-arch (`linux/amd64` + `linux/arm64`)

`linux/arm64` is included from v0.2.0 because a Raspberry Pi (or any ARM host) can be a deployment target — same operator, same Slack workspace, different box. The cost is one extra `rustup target add aarch64-unknown-linux-musl` and a `--platform` flag in the build command; no new tooling. The builder stage pins `--platform=$BUILDPLATFORM`, so the rustc binary itself runs natively on the builder host — only the emitted artifact is cross-compiled. No QEMU is invoked at build time, which keeps the cold-cache budget from REQ-112 realistic. QEMU is invoked only at run time when a developer explicitly pulls a non-host arch (e.g. `docker run --platform linux/arm64` on an `amd64` laptop); operators on native hardware pull the native image and pay no emulation cost. The runtime stage pins `--platform=$TARGETPLATFORM` so `alpine:3.20` resolves to the matching arch and no foreign-architecture layer leaks into the image. The image is published as a single tag backed by an OCI manifest list — `docker pull ghcr.io/ilteoood/slack-wf-trigger:0.2.0` selects the right arch automatically. Other architectures (`linux/arm/v7`, `linux/ppc64le`, `linux/s390x`) remain out of scope; add when a concrete target appears.

### Why no `cargo-chef` in v1

`cargo-chef` is a 30-line diff in the Dockerfile for what `Cargo.toml` dependency-cache invalidation already gives us. The two-pass "fake `main.rs`" trick is functionally equivalent for a project this size. Re-evaluate if a CI rebuild ever exceeds the 90 s warm-budget from REQ-112.

### Why GH Actions on `ubuntu-24.04` with `GITHUB_TOKEN`

GitHub-hosted runners eliminate the runner-maintenance tax (per CON-106). `ubuntu-24.04` ships Docker ≥ 24.0 and BuildKit, satisfying CON-104 and the `# syntax=docker/dockerfile:1.7` line. `GITHUB_TOKEN` is auto-injected and has `packages: write` for `ghcr.io` push on tag workflows — no PAT rotation, no leaked secrets (per CON-107). The trade-off is runner-minute cost, which is acceptable for a single-arch matrix of this size.

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

- **PLT-101**: Rust stable ≥ 1.96.1 (provided by `rust:1.96.1-alpine3.24`).
- **PLT-102**: Alpine Linux 3.20 musl libc (provided by `alpine:3.20`).

### Compliance Dependencies

- None.

## 9. Examples & Edge Cases

### Build

```bash
docker buildx build --platform linux/amd64,linux/arm64 -t slack-wf-trigger:dev .
docker buildx build --platform linux/amd64,linux/arm64 -t ghcr.io/ilteoood/slack-wf-trigger:0.2.0 --push .
```

`--platform` is mandatory for multi-arch: omitting it (or using the legacy `docker build`) produces a single-arch image for the builder host only and breaks the image-index contract. Use `docker buildx` exclusively; the repo's `Dockerfile` is authored for BuildKit (per CON-105).

### Run (smoke test)

```bash
docker run --rm slack-wf-trigger:dev --help
docker run --rm --platform linux/arm64 slack-wf-trigger:dev --help
```

On an `amd64` host, the second command pulls and runs the `arm64` image via QEMU emulation — useful as a sanity check that the cross-compiled binary actually executes, not just builds. Native users on `arm64` get the native image automatically by omitting `--platform`.

### Run (real, `docker compose` — recommended)

```bash
SLACK_USER_TOKEN=xoxp-... docker compose up -d
```

The shipped `docker-compose.yml` wires the recommended layout (read-only `./rules.json` bind-mount, named `slack-wf-trigger-state` volume, cursors-path override, `unless-stopped` restart). Operators only need to provide `SLACK_USER_TOKEN`. Image tag is overridable via `SLACK_WF_TRIGGER_IMAGE_TAG` (default `:0.3.0`).

### Run (real, raw `docker run` — for completeness)

```bash
docker run -d \
  --name slack-wf-trigger \
  --restart unless-stopped \
  -e SLACK_USER_TOKEN=xoxp-... \
  -e RUST_LOG=info,slack_wf_trigger=debug \
  -v $PWD/rules.json:/etc/slack-wf-trigger/rules.json:ro \
  -v slack-wf-trigger-state:/var/lib/slack-wf-trigger \
  -e SLACK_WF_TRIGGER_CURSORS_PATH=/var/lib/slack-wf-trigger/.slack-wf-trigger.cursors.json \
  ghcr.io/ilteoood/slack-wf-trigger:0.3.0
```

The `:0.3.0` tag resolves to an image index; Docker selects `linux/amd64` or `linux/arm64` based on the host. No `--platform` is needed in production.

### Edge cases handled

- `docker run` without `SLACK_USER_TOKEN` → the binary exits non-zero within 5 s with the message required by REQ-010. The container stops; no infinite retry loop. (Operators add `restart: on-failure: 3` if they want bounded retries.)
- Config file is missing inside the container → the binary exits non-zero. `docker run` exit code propagates. No silent polling loop.
- Volume not mounted for cursors file → cursors are written into the container's writable layer. On `docker rm` they're lost. Documented; operator's job to mount a volume.
- `docker stop` during an in-flight triggered command → the binary's signal handler flushes cursors but does NOT kill the spawned subprocess (per main spec's "no command timeout enforcement in v1"). The child may continue running until the container's PID 1 exits; if the kernel reaps the child, it gets SIGKILL on container exit.
- Image pulled with a stale digest → TLS validation fails (`reqwest` rejects the cert); the binary logs a connection error and the operator notices.
- Pulling the image on a host with an unsupported architecture (e.g. `linux/ppc64le`) → Docker fails with a clear manifest-list error rather than silently picking a foreign-arch image. Operator's job to use a supported host or open an issue.
- Cross-arch run on a developer laptop (`docker run --platform linux/arm64` on an `amd64` host) → QEMU emulates the `arm64` binary at run time; it works, but cold-start latency is higher and any per-arch-specific tooling must still resolve correctly. Documented as a developer-only convenience, not a production path.

### Edge cases NOT handled in v1

- Architectures other than `linux/amd64` and `linux/arm64` — see Section 7.
- Auto-restart on transient Slack 5xx — restart policy is the operator's call (`--restart on-failure:5` is a reasonable default but is not baked into the image).
- Config hot reload — main spec already defers this; the image doesn't reopen it.
- Resource limits (`--memory`, `--cpus`) — operator's call.

## 10. Validation Criteria

A change to the Dockerfile, `.dockerignore`, or anything in `src/` or `Cargo.toml` is image-spec-compliant when:

- `docker buildx build --platform linux/amd64,linux/arm64 -t slack-wf-trigger:dev .` succeeds locally.
- `docker run --rm slack-wf-trigger:dev --help` exits 0 on the host's native arch.
- `docker buildx imagetools inspect slack-wf-trigger:dev` lists both `linux/amd64` and `linux/arm64`.
- Compressed image size is under 50 MB per arch (`docker image ls --format='{{.Size}}'` shows the uncompressed size; divide by ~3 for compressed).
- `USER` is `nobody` for every per-arch image (AC-102).
- `ENTRYPOINT` is `[/slack-wf-trigger]` exec form for every per-arch image (AC-103).
- `SSL_CERT_FILE` is set to a path inside the image that exists at runtime (AC-109).
- Any change to `Cargo.toml` without a corresponding `Cargo.lock` update fails CI's `test` job.

## 11. Related Specifications / Further Reading

- `spec/spec-tool-slack-channel-watcher.md` — main spec; this document is its packaging counterpart.
- `listening-to` repository — https://github.com/ilteoood/listening-to — reference pattern for the multi-stage build and `nobody` user; `slack-wf-trigger` diverges from this pattern by using Alpine (not `scratch`) for the runtime base (see Section 7).
- Alpine 3.20 release notes — https://alpinelinux.org/posts/Alpine-3.20.0-released/
- Docker `Dockerfile` reference — https://docs.docker.com/reference/dockerfile/
- `docker stop` semantics — https://docs.docker.com/reference/cli/docker/container/stop/
- GitHub Container Registry — https://docs.github.com/en/packages/working-with-a-github-packages-registry/working-with-the-container-registry
- Rust musl target — https://doc.rust-lang.org/nightly/rustc/platform-support/x86_64-unknown-linux-musl.html
- Rust musl target (arm64) — https://doc.rust-lang.org/nightly/rustc/platform-support/aarch64-unknown-linux-musl.html
- BuildKit multi-platform builds — https://docs.docker.com/build/building/multi-platform/
- `docker buildx imagetools inspect` — https://docs.docker.com/reference/cli/docker/buildx/imagetools/inspect/
- GitHub Actions: publishing Docker images — https://docs.github.com/actions/publishing-packages/publishing-docker-images
- `docker/build-push-action` — https://github.com/docker/build-push-action
- `docker/login-action` — https://github.com/docker/login-action
- `aquasecurity/trivy-action` — https://github.com/aquasecurity/trivy-action
- `reqwest` rustls feature — https://docs.rs/reqwest

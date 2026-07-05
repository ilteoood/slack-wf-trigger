pub mod config;
pub mod cursors;
pub mod mrkdwn;
pub mod slack;
pub mod trigger;

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use tokio::sync::Notify;
use tracing::{info, warn};

use crate::config::Rule;
use crate::cursors::CursorStore;
use crate::slack::{ChannelRef, SlackApi};

pub struct RunArgs {
    pub config_path: PathBuf,
    pub poll_interval: u64,
    pub slack_base_url: Option<String>,
}

struct PollLoopCtx<'a> {
    api: &'a SlackApi,
    store: &'a mut CursorStore,
    channels: &'a [ChannelRef],
    rules: &'a [Rule],
    self_user_id: Option<&'a str>,
    workdir: &'a std::path::Path,
}

pub async fn run(args: RunArgs) -> Result<()> {
    let rules = config::load_rules(&args.config_path)?;
    if rules.is_empty() {
        bail!("config file contains no rules");
    }

    let token = std::env::var("SLACK_USER_TOKEN")
        .context("SLACK_USER_TOKEN environment variable is required")?;
    if token.trim().is_empty() {
        bail!("SLACK_USER_TOKEN environment variable is empty");
    }

    let api = match &args.slack_base_url {
        Some(base) => SlackApi::with_base(token.clone(), base.clone())
            .context("failed to construct Slack client")?,
        None => SlackApi::new(token).context("failed to construct Slack client")?,
    };

    let auth = api
        .auth_test()
        .await
        .context("auth.test failed — is SLACK_USER_TOKEN valid?")?;
    info!(user_id = %auth.user_id, user = %auth.user, team = %auth.team, "authenticated");

    let channels = api
        .list_channels()
        .await
        .context("conversations.list failed")?;
    let watched = resolve_channels(&rules, &channels)?;
    info!(count = watched.len(), "resolved watched channels");

    let workdir = config::config_dir(&args.config_path);
    let mut store = CursorStore::load(&workdir)?;
    if let Some(parent) = store.path().parent()
        && !parent.exists()
    {
        std::fs::create_dir_all(parent).ok();
    }

    let shutdown = Arc::new(Notify::new());
    spawn_signal_handler(shutdown.clone());

    let self_user_id = if auth.user_id.is_empty() {
        None
    } else {
        Some(auth.user_id.as_str())
    };

    let mut ctx = PollLoopCtx {
        api: &api,
        store: &mut store,
        channels: &watched,
        rules: &rules,
        self_user_id,
        workdir: &workdir,
    };
    poll_loop(&mut ctx, args.poll_interval, shutdown.clone()).await?;

    info!("flushing cursors before exit");
    store
        .persist()
        .context("failed to persist cursors on shutdown")?;
    Ok(())
}

async fn poll_loop(
    ctx: &mut PollLoopCtx<'_>,
    interval_secs: u64,
    shutdown: Arc<Notify>,
) -> Result<()> {
    let interval = Duration::from_secs(interval_secs.max(1));

    loop {
        tokio::select! {
            biased;
            _ = shutdown.notified() => {
                info!("shutdown signal received, leaving poll loop");
                break;
            }
            _ = tokio::time::sleep(interval) => {
                for channel in ctx.channels {
                    if let Err(e) = trigger::poll_channel(
                        ctx.api,
                        ctx.store,
                        channel,
                        ctx.rules,
                        ctx.self_user_id,
                        ctx.workdir,
                    )
                    .await
                    {
                        warn!(
                            channel = %channel.id,
                            error = %e,
                            "poll cycle failed for channel"
                        );
                    } else if let Err(e) = ctx.store.persist() {
                        warn!(
                            channel = %channel.id,
                            error = %e,
                            "failed to persist cursors"
                        );
                    }
                }
            }
        }
    }

    Ok(())
}

fn resolve_channels(rules: &[Rule], available: &[ChannelRef]) -> Result<Vec<ChannelRef>> {
    let mut picked: Vec<ChannelRef> = Vec::new();
    let mut missing: HashSet<String> = HashSet::new();

    for rule in rules {
        if let Some(ch) = available.iter().find(|c| c.matches(&rule.channel)) {
            if !picked.iter().any(|p| p.id == ch.id) {
                picked.push(ch.clone());
            }
        } else {
            missing.insert(rule.channel.clone());
        }
    }

    if !missing.is_empty() {
        let mut names: Vec<String> = missing.into_iter().collect();
        names.sort();
        bail!(
            "config references channels not visible to the authenticated user: {}",
            names.join(", ")
        );
    }

    Ok(picked)
}

fn spawn_signal_handler(notify: Arc<Notify>) {
    tokio::spawn(async move {
        #[cfg(unix)]
        {
            use tokio::signal::unix::{SignalKind, signal};
            let mut sigint = match signal(SignalKind::interrupt()) {
                Ok(s) => s,
                Err(e) => {
                    warn!(error = %e, "failed to install SIGINT handler");
                    return;
                }
            };
            let mut sigterm = match signal(SignalKind::terminate()) {
                Ok(s) => s,
                Err(e) => {
                    warn!(error = %e, "failed to install SIGTERM handler");
                    return;
                }
            };
            tokio::select! {
                _ = sigint.recv() => info!("received SIGINT"),
                _ = sigterm.recv() => info!("received SIGTERM"),
            }
        }

        #[cfg(not(unix))]
        {
            let _ = tokio::signal::ctrl_c().await;
            info!("received Ctrl+C");
        }

        notify.notify_one();
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Matcher, Rule};

    fn rule(channel: &str) -> Rule {
        Rule {
            index: 0,
            channel: channel.into(),
            matcher: Matcher::Substring("x".into()),
            command: "echo".into(),
        }
    }

    #[test]
    fn resolve_channels_matches_by_id_and_name() {
        let available = vec![
            ChannelRef {
                id: "C1".into(),
                name: Some("general".into()),
            },
            ChannelRef {
                id: "C2".into(),
                name: Some("alerts".into()),
            },
        ];

        let rules = vec![rule("general"), rule("C2")];
        let resolved = resolve_channels(&rules, &available).unwrap();
        assert_eq!(resolved.len(), 2);
    }

    #[test]
    fn resolve_channels_fails_with_unresolved_list() {
        let available = vec![ChannelRef {
            id: "C1".into(),
            name: Some("general".into()),
        }];
        let rules = vec![rule("general"), rule("missing")];
        let err = resolve_channels(&rules, &available)
            .unwrap_err()
            .to_string();
        assert!(err.contains("missing"), "got: {err}");
    }
}

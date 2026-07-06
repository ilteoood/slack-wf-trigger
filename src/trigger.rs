use std::path::Path;
use std::process::Stdio;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use tokio::process::Command;
use tracing::{info, warn};

use crate::config::{Matcher, Rule};
use crate::cursors::CursorStore;
use crate::mrkdwn;
use crate::slack::{ChannelRef, RawMessage, SlackApi};

pub const STDOUT_STDERR_LOG_LIMIT: usize = 4 * 1024;
pub const COMMAND_TIMEOUT_WARNING: Duration = Duration::from_secs(5 * 60);

#[derive(Debug, Clone)]
pub struct MessageContext {
    pub channel_id: String,
    pub channel_name: Option<String>,
    pub user: Option<String>,
    pub text: String,
    pub ts: String,
}

impl MessageContext {
    pub fn from_raw(
        raw: &RawMessage,
        channel_id: &str,
        channel_name: Option<&str>,
    ) -> Option<Self> {
        if raw.ts.is_empty() {
            return None;
        }
        Some(Self {
            channel_id: channel_id.to_owned(),
            channel_name: channel_name.map(str::to_owned),
            user: raw.author().map(str::to_owned),
            text: raw.text.clone(),
            ts: raw.ts.clone(),
        })
    }
}

pub fn matches(rule: &Rule, text: &str) -> bool {
    let stripped = mrkdwn::strip(text);
    match &rule.matcher {
        Matcher::Substring(needle) => stripped.contains(needle.as_str()),
        Matcher::Regex(re) => re.is_match(&stripped),
    }
}

pub fn rules_for_channel<'a>(
    rules: &'a [Rule],
    channels: &[ChannelRef],
    channel: &str,
) -> Vec<&'a Rule> {
    let channel_refs = channels
        .iter()
        .filter(|c| c.matches(channel))
        .collect::<Vec<_>>();
    if channel_refs.is_empty() {
        return Vec::new();
    }

    rules
        .iter()
        .filter(|rule| channel_refs.iter().any(|c| c.matches(&rule.channel)))
        .collect()
}

pub async fn process_message(
    api: &SlackApi,
    workdir: &Path,
    msg: &MessageContext,
    matched: &[&Rule],
) {
    add_reaction_quietly(api, &msg.channel_id, &msg.ts, "thumbsup").await;

    for rule in matched {
        let outcome = run_command(rule, msg, workdir).await;
        match outcome {
            Ok(out) if out.exit_code == 0 => {
                info!(
                    rule_index = rule.index,
                    command = %rule.command,
                    exit_code = 0,
                    duration_ms = out.duration.as_millis() as u64,
                    "command succeeded"
                );
                if !out.stdout.is_empty() {
                    log_tail("stdout", &out.stdout);
                }
                if !out.stderr.is_empty() {
                    log_tail("stderr", &out.stderr);
                }
                add_reaction_quietly(api, &msg.channel_id, &msg.ts, "white_check_mark").await;
            }
            Ok(out) => {
                warn!(
                    rule_index = rule.index,
                    command = %rule.command,
                    exit_code = out.exit_code,
                    duration_ms = out.duration.as_millis() as u64,
                    "command exited non-zero"
                );
                if !out.stdout.is_empty() {
                    log_tail("stdout", &out.stdout);
                }
                if !out.stderr.is_empty() {
                    log_tail("stderr", &out.stderr);
                }
                add_reaction_quietly(api, &msg.channel_id, &msg.ts, "x").await;
            }
            Err(e) => {
                warn!(
                    rule_index = rule.index,
                    command = %rule.command,
                    error = %e,
                    "command failed to spawn"
                );
                add_reaction_quietly(api, &msg.channel_id, &msg.ts, "x").await;
            }
        }
    }
}

#[derive(Debug)]
pub struct CommandOutcome {
    pub exit_code: i32,
    pub duration: Duration,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

pub async fn run_command(
    rule: &Rule,
    msg: &MessageContext,
    workdir: &Path,
) -> Result<CommandOutcome> {
    let started = Instant::now();
    let mut cmd = Command::new("sh");
    cmd.arg("-c")
        .arg(&rule.command)
        .current_dir(workdir)
        .env(
            "SLACK_WF_TRIGGER_CHANNEL",
            msg.channel_name.as_deref().unwrap_or(&msg.channel_id),
        )
        .env("SLACK_WF_TRIGGER_CHANNEL_ID", &msg.channel_id)
        .env(
            "SLACK_WF_TRIGGER_USER",
            msg.user.clone().unwrap_or_default(),
        )
        .env("SLACK_WF_TRIGGER_TEXT", &msg.text)
        .env("SLACK_WF_TRIGGER_TS", &msg.ts)
        .env("SLACK_WF_TRIGGER_RULE_INDEX", rule.index.to_string())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let child = cmd
        .spawn()
        .with_context(|| format!("failed to spawn command for rule {}", rule.index))?;

    let output = child
        .wait_with_output()
        .await
        .context("failed waiting for command output")?;

    let duration = started.elapsed();
    if duration >= COMMAND_TIMEOUT_WARNING {
        warn!(
            rule_index = rule.index,
            duration_secs = duration.as_secs(),
            "command exceeded soft timeout warning (still running? see spec §7)"
        );
    }

    Ok(CommandOutcome {
        exit_code: output.status.code().unwrap_or(-1),
        duration,
        stdout: tail(&output.stdout, STDOUT_STDERR_LOG_LIMIT),
        stderr: tail(&output.stderr, STDOUT_STDERR_LOG_LIMIT),
    })
}

async fn add_reaction_quietly(api: &SlackApi, channel: &str, ts: &str, name: &str) {
    if let Err(e) = api.add_reaction(channel, ts, name).await {
        warn!(channel, ts, name, error = %e, "reactions.add failed; continuing");
    }
}

fn tail(bytes: &[u8], limit: usize) -> Vec<u8> {
    if bytes.len() <= limit {
        bytes.to_vec()
    } else {
        bytes[bytes.len() - limit..].to_vec()
    }
}

fn log_tail(label: &str, bytes: &[u8]) {
    let text = String::from_utf8_lossy(bytes);
    for line in text.lines() {
        info!(stream = label, "{}", line);
    }
}

pub async fn poll_channel(
    api: &SlackApi,
    store: &mut CursorStore,
    channel: &ChannelRef,
    rules: &[Rule],
    workdir: &Path,
) -> Result<()> {
    let cursor = store.get(&channel.id).cloned();
    let raw_messages = api
        .history(&channel.id, cursor.as_deref())
        .await
        .with_context(|| format!("history fetch failed for channel {}", channel.id))?;

    if raw_messages.is_empty() {
        return Ok(());
    }

    let Some(newest_ts) = raw_messages.last().map(|m| m.ts.clone()) else {
        return Ok(());
    };

    let is_seed = cursor.is_none();
    let cursor_ts = cursor.as_deref().unwrap_or("");

    if is_seed {
        info!(
            channel = %channel.id,
            count = raw_messages.len(),
            newest_ts = %newest_ts,
            "seeding cursor from initial poll; no commands run on seed"
        );
        store.set(channel.id.clone(), newest_ts);
        return Ok(());
    }

    for raw in &raw_messages {
        if raw.ts.as_str() <= cursor_ts {
            continue;
        }
        let Some(msg) = MessageContext::from_raw(raw, &channel.id, channel.name.as_deref()) else {
            continue;
        };

        let matched: Vec<&Rule> = rules
            .iter()
            .filter(|r| channel.matches(&r.channel))
            .filter(|r| matches(r, &msg.text))
            .collect();

        if !matched.is_empty() {
            info!(
                channel = %msg.channel_id,
                ts = %msg.ts,
                user = msg.user.as_deref().unwrap_or(""),
                matched_rules = matched.len(),
                "message matched"
            );
            process_message(api, workdir, &msg, &matched).await;
        }

        store.set(channel.id.clone(), msg.ts.clone());
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Matcher, Rule};
    use regex::Regex;

    fn rule(channel: &str, message: &str, regex: bool, command: &str) -> Rule {
        let matcher = if regex {
            Matcher::Regex(Regex::new(message).unwrap())
        } else {
            Matcher::Substring(message.into())
        };
        Rule {
            index: 0,
            channel: channel.into(),
            matcher,
            command: command.into(),
        }
    }

    #[test]
    fn substring_matches_after_mrkdwn_stripping() {
        let r = rule("general", "deploy prod", false, "echo");
        assert!(matches(&r, "please deploy prod now"));
        assert!(matches(&r, "see <https://x|deploy prod>"));
        assert!(!matches(&r, "deploy dev"));
    }

    #[test]
    fn regex_match_works() {
        let r = rule("general", r"^deploy (prod|staging)$", true, "echo");
        assert!(matches(&r, "deploy prod"));
        assert!(matches(&r, "deploy staging"));
        assert!(!matches(&r, "deploy dev"));
    }

    #[test]
    fn rules_for_channel_matches_by_id_and_name() {
        let channels = vec![
            ChannelRef {
                id: "C1".into(),
                name: Some("general".into()),
            },
            ChannelRef {
                id: "C2".into(),
                name: Some("alerts".into()),
            },
        ];

        let rules = vec![
            rule("general", "x", false, "echo"),
            rule("C2", "y", false, "echo"),
            rule("random", "z", false, "echo"),
        ];

        let picked = rules_for_channel(&rules, &channels, "general");
        assert_eq!(picked.len(), 1);
        assert_eq!(picked[0].channel, "general");

        let picked_by_id = rules_for_channel(&rules, &channels, "C2");
        assert_eq!(picked_by_id.len(), 1);
        assert_eq!(picked_by_id[0].channel, "C2");

        let picked_none = rules_for_channel(&rules, &channels, "missing");
        assert!(picked_none.is_empty());
    }

    #[tokio::test]
    async fn env_vars_injected_into_command() {
        let dir = tempfile::tempdir().unwrap();
        let r = rule(
            "general",
            "x",
            false,
            "echo \"$SLACK_WF_TRIGGER_TEXT|$SLACK_WF_TRIGGER_CHANNEL|$SLACK_WF_TRIGGER_CHANNEL_ID|$SLACK_WF_TRIGGER_USER|$SLACK_WF_TRIGGER_TS|$SLACK_WF_TRIGGER_RULE_INDEX\" > env.log",
        );
        let msg = MessageContext {
            channel_id: "C1".into(),
            channel_name: Some("general".into()),
            user: Some("U42".into()),
            text: "hello world".into(),
            ts: "1700000000.000100".into(),
        };

        let outcome = run_command(&r, &msg, dir.path()).await.unwrap();
        assert_eq!(outcome.exit_code, 0);

        let log = std::fs::read_to_string(dir.path().join("env.log")).unwrap();
        assert_eq!(log.trim(), "hello world|general|C1|U42|1700000000.000100|0");
    }

    #[tokio::test]
    async fn non_zero_exit_captured() {
        let dir = tempfile::tempdir().unwrap();
        let r = rule("c", "x", false, "sh -c 'echo bad 1>&2; exit 7'");
        let msg = MessageContext {
            channel_id: "C1".into(),
            channel_name: None,
            user: Some("U".into()),
            text: "x".into(),
            ts: "1.0".into(),
        };
        let outcome = run_command(&r, &msg, dir.path()).await.unwrap();
        assert_eq!(outcome.exit_code, 7);
        let stderr = String::from_utf8_lossy(&outcome.stderr);
        assert!(stderr.contains("bad"));
    }
}

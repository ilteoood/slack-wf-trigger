use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use serde::Deserialize;
use tracing::{debug, warn};

const DEFAULT_BASE_URL: &str = "https://slack.com/api";
const HISTORY_PAGE_SIZE: u32 = 100;
const MAX_POLL_RETRIES: u32 = 3;

#[derive(Debug, Clone)]
pub struct SlackApi {
    http: reqwest::Client,
    token: String,
    base: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AuthTestResponse {
    #[serde(default)]
    pub user_id: String,
    #[serde(default)]
    pub user: String,
    #[serde(default)]
    pub team: String,
    #[serde(default)]
    pub team_id: String,
}

#[derive(Debug, Clone)]
pub struct ChannelRef {
    pub id: String,
    pub name: Option<String>,
}

impl ChannelRef {
    pub fn matches(&self, rule_channel: &str) -> bool {
        self.id == rule_channel || self.name.as_deref().is_some_and(|n| n == rule_channel)
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct RawMessage {
    #[serde(default, rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub user: Option<String>,
    #[serde(default)]
    pub bot_id: Option<String>,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub ts: String,
    #[serde(default)]
    pub thread_ts: String,
}

impl RawMessage {
    pub fn author(&self) -> Option<&str> {
        self.user
            .as_deref()
            .or(self.bot_id.as_deref())
            .filter(|s| !s.is_empty())
    }
}

impl SlackApi {
    pub fn new(token: impl Into<String>) -> Result<Self> {
        Self::with_base(token, DEFAULT_BASE_URL.to_owned())
    }

    pub fn with_base(token: impl Into<String>, base: String) -> Result<Self> {
        let http = reqwest::Client::builder()
            .user_agent(format!("wf-trigger/{}", env!("CARGO_PKG_VERSION")))
            .timeout(Duration::from_secs(30))
            .build()
            .context("failed to build HTTP client")?;
        Ok(Self {
            http,
            token: token.into(),
            base,
        })
    }

    pub async fn auth_test(&self) -> Result<AuthTestResponse> {
        let body: ApiResponse<AuthTestResponse> = self
            .call("auth.test", &[])
            .await
            .context("auth.test request failed")?;
        body.into_data()
    }

    pub async fn list_channels(&self) -> Result<Vec<ChannelRef>> {
        let mut all: Vec<ChannelRef> = Vec::new();
        let mut cursor: Option<String> = None;

        loop {
            let mut params: Vec<(&str, String)> = vec![
                ("types", "public_channel,private_channel,mpim,im".into()),
                ("exclude_archived", "true".into()),
                ("limit", "200".into()),
            ];
            if let Some(c) = &cursor {
                params.push(("cursor", c.clone()));
            }

            let body: ApiResponse<ChannelsResponse> = self
                .call("conversations.list", &params)
                .await
                .context("conversations.list request failed")?;
            let data = body.into_data()?;
            for ch in data.channels {
                let name = ch.name.filter(|n| !n.is_empty());
                all.push(ChannelRef { id: ch.id, name });
            }
            match data.response_metadata.and_then(|m| m.next_cursor) {
                Some(c) if !c.is_empty() => cursor = Some(c),
                _ => break,
            }
        }

        Ok(all)
    }

    pub async fn history(&self, channel_id: &str, oldest: Option<&str>) -> Result<Vec<RawMessage>> {
        let mut all: Vec<RawMessage> = Vec::new();
        let mut cursor_oldest = oldest.map(|s| s.to_owned());

        loop {
            let mut params: Vec<(&str, String)> = vec![
                ("channel", channel_id.to_owned()),
                ("limit", HISTORY_PAGE_SIZE.to_string()),
            ];
            if let Some(o) = &cursor_oldest {
                params.push(("oldest", o.clone()));
            }
            if let Some(last) = all.last() {
                params.push(("latest", last.ts.clone()));
            }

            let body: ApiResponse<HistoryResponse> = self
                .call("conversations.history", &params)
                .await
                .context("conversations.history request failed")?;
            let data = body.into_data()?;

            let count_before = all.len();
            for m in data.messages {
                if m.kind != "message" {
                    continue;
                }
                all.push(m);
            }

            let reached_page_end = all.len() - count_before < HISTORY_PAGE_SIZE as usize;
            if data.has_more.unwrap_or(false) && !reached_page_end {
                if let Some(last) = all.last() {
                    cursor_oldest = Some(last.ts.clone());
                }
            } else {
                break;
            }
        }

        Ok(all)
    }

    pub async fn add_reaction(&self, channel_id: &str, ts: &str, name: &str) -> Result<()> {
        let params = [
            ("channel", channel_id.to_owned()),
            ("timestamp", ts.to_owned()),
            ("name", name.to_owned()),
        ];

        match self
            .call::<ReactionResponse>("reactions.add", &params)
            .await
        {
            Ok(body) => match body.into_data() {
                Ok(_) => Ok(()),
                Err(e) if e.to_string().contains("already_reacted") => {
                    debug!(channel = channel_id, ts, name, "reaction already present");
                    Ok(())
                }
                Err(e) => Err(e),
            },
            Err(e) => {
                let msg = e.to_string();
                if msg.contains("already_reacted") {
                    debug!(channel = channel_id, ts, name, "reaction already present");
                    Ok(())
                } else {
                    Err(e)
                }
            }
        }
    }

    async fn call<T: for<'de> Deserialize<'de>>(
        &self,
        method: &str,
        params: &[(&str, String)],
    ) -> Result<ApiResponse<T>> {
        let url = format!("{}/{}", self.base, method);
        let mut attempt = 0u32;

        loop {
            attempt += 1;
            let response = self
                .http
                .post(&url)
                .bearer_auth(&self.token)
                .form(params)
                .send()
                .await;

            match response {
                Ok(resp) => {
                    let status = resp.status();
                    if status == reqwest::StatusCode::TOO_MANY_REQUESTS
                        && attempt <= MAX_POLL_RETRIES
                    {
                        let retry_after = resp
                            .headers()
                            .get("retry-after")
                            .and_then(|v| v.to_str().ok())
                            .and_then(|s| s.parse::<u64>().ok())
                            .unwrap_or(5);
                        warn!(method, retry_after, "rate limited, sleeping");
                        tokio::time::sleep(Duration::from_secs(retry_after.min(60))).await;
                        continue;
                    }

                    let bytes = resp
                        .bytes()
                        .await
                        .with_context(|| format!("failed to read body for {method}"))?;
                    let parsed: ApiResponse<T> = serde_json::from_slice(&bytes)
                        .with_context(|| format!("failed to decode {method} response"))?;
                    return Ok(parsed);
                }
                Err(e) if attempt <= MAX_POLL_RETRIES => {
                    let delay = Duration::from_secs(2u64.pow(attempt.min(5)));
                    warn!(method, error = %e, ?delay, "transient Slack API error, retrying");
                    tokio::time::sleep(delay).await;
                    continue;
                }
                Err(e) => {
                    return Err(anyhow!(e)).with_context(|| format!("{method} HTTP error"));
                }
            }
        }
    }
}

#[derive(Debug, Deserialize)]
struct ApiResponse<T> {
    #[serde(default)]
    ok: bool,
    #[serde(default)]
    error: Option<String>,
    #[serde(flatten)]
    data: Option<T>,
}

impl<T> ApiResponse<T> {
    fn into_data(self) -> Result<T> {
        if self.ok {
            self.data
                .ok_or_else(|| anyhow!("Slack response marked ok but missing payload"))
        } else {
            let err = self.error.unwrap_or_else(|| "unknown_error".to_owned());
            bail!("Slack API error: {err}");
        }
    }
}

#[derive(Debug, Deserialize)]
struct ChannelsResponse {
    #[serde(default)]
    channels: Vec<RawChannel>,
    #[serde(default)]
    response_metadata: Option<CursorMeta>,
}

#[derive(Debug, Deserialize)]
struct RawChannel {
    id: String,
    #[serde(default)]
    name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CursorMeta {
    #[serde(default)]
    next_cursor: Option<String>,
}

#[derive(Debug, Deserialize)]
struct HistoryResponse {
    #[serde(default)]
    messages: Vec<RawMessage>,
    #[serde(default)]
    has_more: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct ReactionResponse {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channel_ref_matches_by_id_or_name() {
        let ch = ChannelRef {
            id: "C0123".into(),
            name: Some("general".into()),
        };
        assert!(ch.matches("C0123"));
        assert!(ch.matches("general"));
        assert!(!ch.matches("C9999"));
        assert!(!ch.matches("other"));
    }

    #[test]
    fn channel_ref_with_no_name_matches_only_by_id() {
        let ch = ChannelRef {
            id: "D0123".into(),
            name: None,
        };
        assert!(ch.matches("D0123"));
        assert!(!ch.matches("anything"));
    }
}

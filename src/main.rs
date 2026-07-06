use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;

use slack_wf_trigger::RunArgs;

#[derive(Debug, Parser)]
#[command(
    name = "slack-wf-trigger",
    version,
    about = "Watch Slack channels for matching messages and run shell commands",
    long_about = "slack-wf-trigger polls a single Slack workspace via the Web API, evaluates incoming \
messages against a JSON rule list, and runs a configured shell command per match.\n\n\
SECURITY: triggered commands are passed to `sh -c` with no sandboxing. The operator is \
trusted. Do not run this as root or on a multi-user host.\n\n\
Required Slack user token scopes: channels:history, groups:history, channels:read, \
groups:read, reactions:write, users:read. Optional: im:history, mpim:history, im:read, \
mpim:read."
)]
struct Cli {
    #[arg(long, env = "SLACK_WF_TRIGGER_CONFIG", value_name = "PATH")]
    config: Option<PathBuf>,

    #[arg(long, env = "SLACK_TOKEN", hide_env = true)]
    slack_token: String,

    #[arg(long, env = "SLACK_COOKIE", hide_env = true)]
    slack_cookie: Option<String>,

    #[arg(long, env = "SLACK_BASE_URL", default_value = "https://slack.com/api")]
    slack_base_url: String,

    #[arg(
        long,
        env = "SLACK_WF_TRIGGER_POLL_INTERVAL",
        value_name = "SECS",
        default_value_t = 10,
        value_parser = clap::value_parser!(u64).range(1..)
    )]
    poll_interval: u64,
}

#[tokio::main]
async fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_target(false)
        .init();

    let cli = Cli::parse();

    let config_path = match cli.config {
        Some(p) => p,
        None => {
            eprintln!(
                "error: missing --config <PATH> (or SLACK_WF_TRIGGER_CONFIG env var); \
                 nothing to do"
            );
            return ExitCode::from(1);
        }
    };

    let cookie = cli
        .slack_cookie
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty());

    let args = RunArgs {
        config_path,
        poll_interval: cli.poll_interval,
        slack_token: cli.slack_token,
        slack_base_url: cli.slack_base_url,
        slack_cookie: cookie,
    };

    match slack_wf_trigger::run(args).await {
        Ok(()) => ExitCode::from(0),
        Err(e) => {
            eprintln!("error: {e:#}");
            e.chain().skip(1).for_each(|cause| {
                eprintln!("  caused by: {cause}");
            });
            tracing::error!(error = ?e, "fatal");
            ExitCode::from(1)
        }
    }
}

use std::path::Path;

use serde_json::json;
use tempfile::TempDir;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use slack_wf_trigger::config::{self, Matcher, Rule};
use slack_wf_trigger::cursors::CursorStore;
use slack_wf_trigger::slack::{ChannelRef, SlackApi};
use slack_wf_trigger::trigger;

fn rule(channel: &str, message: &str, regex: bool, command: &str, index: usize) -> Rule {
    let matcher = if regex {
        Matcher::Regex(regex::Regex::new(message).unwrap())
    } else {
        Matcher::Substring(message.into())
    };
    Rule {
        index,
        channel: channel.into(),
        matcher,
        command: command.into(),
    }
}

fn write_config(dir: &Path, body: &str) -> std::path::PathBuf {
    let path = dir.join("rules.json");
    std::fs::write(&path, body).unwrap();
    path
}

fn write_cursors(dir: &Path, body: &str) {
    std::fs::write(dir.join(".slack-wf-trigger.cursors.json"), body).unwrap();
}

fn auth_test_response(user_id: &str) -> serde_json::Value {
    json!({
        "ok": true,
        "user_id": user_id,
        "user": "tester",
        "team": "T0123",
        "team_id": "T0123"
    })
}

fn channel_list(channels: &[(&str, &str)]) -> serde_json::Value {
    let list: Vec<_> = channels
        .iter()
        .map(|(id, name)| {
            json!({
                "id": id,
                "name": name,
                "is_member": true,
            })
        })
        .collect();
    json!({
        "ok": true,
        "channels": list,
        "response_metadata": { "next_cursor": "" }
    })
}

fn history_response(messages: &[(&str, &str, &str)]) -> serde_json::Value {
    let list: Vec<_> = messages
        .iter()
        .map(|(user, text, ts)| {
            json!({
                "type": "message",
                "user": user,
                "text": text,
                "ts": ts,
            })
        })
        .collect();
    json!({
        "ok": true,
        "messages": list,
        "has_more": false
    })
}

fn ok_response() -> serde_json::Value {
    json!({"ok": true})
}

#[tokio::test]
async fn ac001_happy_path_thumbsup_before_command_white_check_on_success() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/auth.test"))
        .respond_with(ResponseTemplate::new(200).set_body_json(auth_test_response("U_OPERATOR")))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/conversations.list"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(channel_list(&[("C_GENERAL", "general")])),
        )
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/conversations.history"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(history_response(&[(
                "U_OTHER",
                "please ping me",
                "1717600042.000456",
            )])),
        )
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/reactions.add"))
        .respond_with(ResponseTemplate::new(200).set_body_json(ok_response()))
        .expect(2)
        .mount(&server)
        .await;

    let dir = TempDir::new().unwrap();
    write_cursors(dir.path(), r#"{"C_GENERAL":"1717600042.000000"}"#);
    let api = SlackApi::with_base("xoxp-test", server.uri()).unwrap();

    let rules = vec![rule("general", "ping", false, "echo pong > out.log", 0)];

    let mut store = CursorStore::load(dir.path()).unwrap();
    let channel = ChannelRef {
        id: "C_GENERAL".into(),
        name: Some("general".into()),
    };

    trigger::poll_channel(
        &api,
        &mut store,
        &channel,
        &rules,
        Some("U_OPERATOR"),
        dir.path(),
    )
    .await
    .unwrap();
    store.persist().unwrap();

    assert!(dir.path().join("out.log").exists(), "command did not spawn");

    let requests = server.received_requests().await.unwrap();
    let call_sequence: Vec<&str> = requests
        .iter()
        .map(|r| r.url.path().trim_start_matches('/').to_string().leak() as &str)
        .collect();
    let _ = call_sequence;

    let add_calls: Vec<&wiremock::Request> = requests
        .iter()
        .filter(|r| r.url.path() == "/reactions.add")
        .collect();
    assert_eq!(add_calls.len(), 2, "expected 2 reaction.add calls");

    let first_body: String = String::from_utf8_lossy(&add_calls[0].body).into_owned();
    let second_body: String = String::from_utf8_lossy(&add_calls[1].body).into_owned();
    assert!(
        first_body.contains("thumbsup"),
        "first reaction should be thumbsup, got: {first_body}"
    );
    assert!(
        second_body.contains("white_check_mark"),
        "second reaction should be white_check_mark, got: {second_body}"
    );

    let reload = CursorStore::load(dir.path()).unwrap();
    assert_eq!(
        reload.get("C_GENERAL"),
        Some(&"1717600042.000456".to_string())
    );
}

#[tokio::test]
async fn ac002_cursor_filters_old_messages() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/auth.test"))
        .respond_with(ResponseTemplate::new(200).set_body_json(auth_test_response("U_OP")))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/conversations.list"))
        .respond_with(ResponseTemplate::new(200).set_body_json(channel_list(&[("C0", "general")])))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/conversations.history"))
        .respond_with(ResponseTemplate::new(200).set_body_json(history_response(&[
            ("U1", "old ping one", "1717600041.000001"),
            ("U1", "old ping two", "1717600041.000002"),
            ("U1", "new ping", "1717600043.000100"),
        ])))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/reactions.add"))
        .respond_with(ResponseTemplate::new(200).set_body_json(ok_response()))
        .expect(2)
        .mount(&server)
        .await;

    let dir = TempDir::new().unwrap();
    write_cursors(dir.path(), r#"{"C0":"1717600042.000000"}"#);
    let api = SlackApi::with_base("xoxp-test", server.uri()).unwrap();
    let rules = vec![rule("general", "ping", false, "echo hit > out.log", 0)];

    let mut store = CursorStore::load(dir.path()).unwrap();
    let channel = ChannelRef {
        id: "C0".into(),
        name: Some("general".into()),
    };

    trigger::poll_channel(&api, &mut store, &channel, &rules, Some("U_OP"), dir.path())
        .await
        .unwrap();

    let log = std::fs::read_to_string(dir.path().join("out.log")).unwrap_or_default();
    assert_eq!(log.lines().filter(|l| l.contains("hit")).count(), 1);
}

#[tokio::test]
async fn ac003_two_rules_for_same_channel_spawn_both_in_order() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/auth.test"))
        .respond_with(ResponseTemplate::new(200).set_body_json(auth_test_response("U_OP")))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/conversations.list"))
        .respond_with(ResponseTemplate::new(200).set_body_json(channel_list(&[("C0", "general")])))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/conversations.history"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(history_response(&[(
                "U1",
                "build green & release ",
                "1717600045.000100",
            )])),
        )
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/reactions.add"))
        .respond_with(ResponseTemplate::new(200).set_body_json(ok_response()))
        .mount(&server)
        .await;

    let dir = TempDir::new().unwrap();
    write_cursors(dir.path(), r#"{"C0":"1717600040.000000"}"#);
    let api = SlackApi::with_base("xoxp-test", server.uri()).unwrap();
    let rules = vec![
        rule("general", "build green", false, "echo a >> out.log", 0),
        rule("general", "release ", false, "echo b >> out.log", 1),
    ];

    let mut store = CursorStore::load(dir.path()).unwrap();
    let channel = ChannelRef {
        id: "C0".into(),
        name: Some("general".into()),
    };
    trigger::poll_channel(&api, &mut store, &channel, &rules, Some("U_OP"), dir.path())
        .await
        .unwrap();

    let log = std::fs::read_to_string(dir.path().join("out.log")).unwrap();
    let lines: Vec<&str> = log.lines().collect();
    assert_eq!(lines, vec!["a", "b"], "rules should run in order");
}

#[tokio::test]
async fn ac014_self_authored_message_is_ignored() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/auth.test"))
        .respond_with(ResponseTemplate::new(200).set_body_json(auth_test_response("U_SELF")))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/conversations.list"))
        .respond_with(ResponseTemplate::new(200).set_body_json(channel_list(&[("C0", "general")])))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/conversations.history"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(history_response(&[(
                "U_SELF",
                "please ping me",
                "1717600045.000100",
            )])),
        )
        .mount(&server)
        .await;

    let dir = TempDir::new().unwrap();
    write_cursors(dir.path(), r#"{"C0":"1717600040.000000"}"#);
    let api = SlackApi::with_base("xoxp-test", server.uri()).unwrap();
    let rules = vec![rule("general", "ping", false, "echo hit > out.log", 0)];

    let mut store = CursorStore::load(dir.path()).unwrap();
    let channel = ChannelRef {
        id: "C0".into(),
        name: Some("general".into()),
    };
    trigger::poll_channel(
        &api,
        &mut store,
        &channel,
        &rules,
        Some("U_SELF"),
        dir.path(),
    )
    .await
    .unwrap();

    assert!(
        !dir.path().join("out.log").exists(),
        "command should not have run for self-authored message"
    );

    let requests = server.received_requests().await.unwrap();
    let reactions = requests
        .iter()
        .filter(|r| r.url.path() == "/reactions.add")
        .count();
    assert_eq!(reactions, 0, "no reactions for self-authored message");
}

#[tokio::test]
async fn ac013_failure_reaction_added_on_non_zero_exit() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/auth.test"))
        .respond_with(ResponseTemplate::new(200).set_body_json(auth_test_response("U_OP")))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/conversations.list"))
        .respond_with(ResponseTemplate::new(200).set_body_json(channel_list(&[("C0", "general")])))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/conversations.history"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(history_response(&[(
                "U1",
                "deploy me",
                "1717600045.000100",
            )])),
        )
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/reactions.add"))
        .respond_with(ResponseTemplate::new(200).set_body_json(ok_response()))
        .expect(2)
        .mount(&server)
        .await;

    let dir = TempDir::new().unwrap();
    write_cursors(dir.path(), r#"{"C0":"1717600040.000000"}"#);
    let api = SlackApi::with_base("xoxp-test", server.uri()).unwrap();
    let rules = vec![rule("general", "deploy", false, "sh -c 'exit 42'", 0)];

    let mut store = CursorStore::load(dir.path()).unwrap();
    let channel = ChannelRef {
        id: "C0".into(),
        name: Some("general".into()),
    };
    trigger::poll_channel(&api, &mut store, &channel, &rules, Some("U_OP"), dir.path())
        .await
        .unwrap();

    let requests = server.received_requests().await.unwrap();
    let reactions: Vec<String> = requests
        .iter()
        .filter(|r| r.url.path() == "/reactions.add")
        .map(|r| String::from_utf8_lossy(&r.body).into_owned())
        .collect();
    assert_eq!(reactions.len(), 2);
    assert!(reactions[0].contains("thumbsup"));
    assert!(reactions[1].contains("x"));
}

#[tokio::test]
async fn ac015_already_reacted_treated_as_success() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/auth.test"))
        .respond_with(ResponseTemplate::new(200).set_body_json(auth_test_response("U_OP")))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/conversations.list"))
        .respond_with(ResponseTemplate::new(200).set_body_json(channel_list(&[("C0", "general")])))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/conversations.history"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(history_response(&[(
                "U1",
                "ping please",
                "1717600045.000100",
            )])),
        )
        .mount(&server)
        .await;

    Mock::given(method("POST"))
        .and(path("/reactions.add"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "ok": false,
            "error": "already_reacted"
        })))
        .mount(&server)
        .await;

    let dir = TempDir::new().unwrap();
    write_cursors(dir.path(), r#"{"C0":"1717600040.000000"}"#);
    let api = SlackApi::with_base("xoxp-test", server.uri()).unwrap();
    let rules = vec![rule("general", "ping", false, "echo ok > out.log", 0)];

    let mut store = CursorStore::load(dir.path()).unwrap();
    let channel = ChannelRef {
        id: "C0".into(),
        name: Some("general".into()),
    };
    let result =
        trigger::poll_channel(&api, &mut store, &channel, &rules, Some("U_OP"), dir.path()).await;
    assert!(
        result.is_ok(),
        "already_reacted must not fail poll: {:?}",
        result.err()
    );

    assert!(
        dir.path().join("out.log").exists(),
        "command should still run"
    );
}

#[tokio::test]
async fn ac016_reaction_failure_does_not_block_command() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/auth.test"))
        .respond_with(ResponseTemplate::new(200).set_body_json(auth_test_response("U_OP")))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/conversations.list"))
        .respond_with(ResponseTemplate::new(200).set_body_json(channel_list(&[("C0", "general")])))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/conversations.history"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(history_response(&[(
                "U1",
                "ping please",
                "1717600045.000100",
            )])),
        )
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/reactions.add"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "ok": false,
            "error": "missing_scope"
        })))
        .mount(&server)
        .await;

    let dir = TempDir::new().unwrap();
    write_cursors(dir.path(), r#"{"C0":"1717600040.000000"}"#);
    let api = SlackApi::with_base("xoxp-test", server.uri()).unwrap();
    let rules = vec![rule("general", "ping", false, "echo ok > out.log", 0)];

    let mut store = CursorStore::load(dir.path()).unwrap();
    let channel = ChannelRef {
        id: "C0".into(),
        name: Some("general".into()),
    };
    trigger::poll_channel(&api, &mut store, &channel, &rules, Some("U_OP"), dir.path())
        .await
        .unwrap();

    assert!(
        dir.path().join("out.log").exists(),
        "command should run despite reaction failure"
    );
}

#[tokio::test]
async fn ac017_fresh_install_seeds_cursor_without_spawning_commands() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/auth.test"))
        .respond_with(ResponseTemplate::new(200).set_body_json(auth_test_response("U_OP")))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/conversations.list"))
        .respond_with(ResponseTemplate::new(200).set_body_json(channel_list(&[("C0", "general")])))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/conversations.history"))
        .respond_with(ResponseTemplate::new(200).set_body_json(history_response(&[
            ("U1", "ping one", "1717600041.000001"),
            ("U1", "ping two", "1717600045.000100"),
        ])))
        .mount(&server)
        .await;

    let dir = TempDir::new().unwrap();
    assert!(!dir.path().join(".slack-wf-trigger.cursors.json").exists());
    let api = SlackApi::with_base("xoxp-test", server.uri()).unwrap();
    let rules = vec![rule("general", "ping", false, "echo nope > out.log", 0)];

    let mut store = CursorStore::load(dir.path()).unwrap();
    let channel = ChannelRef {
        id: "C0".into(),
        name: Some("general".into()),
    };
    trigger::poll_channel(&api, &mut store, &channel, &rules, Some("U_OP"), dir.path())
        .await
        .unwrap();
    store.persist().unwrap();

    assert!(
        !dir.path().join("out.log").exists(),
        "no command should spawn on seed"
    );
    let reload = CursorStore::load(dir.path()).unwrap();
    assert_eq!(reload.get("C0"), Some(&"1717600045.000100".to_string()));
}

#[tokio::test]
async fn ac004_regex_accepts_prod() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/auth.test"))
        .respond_with(ResponseTemplate::new(200).set_body_json(auth_test_response("U_OP")))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/conversations.list"))
        .respond_with(ResponseTemplate::new(200).set_body_json(channel_list(&[("C0", "general")])))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/conversations.history"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(history_response(&[(
                "U1",
                "deploy prod",
                "1717600047.000100",
            )])),
        )
        .mount(&server)
        .await;

    let dir = TempDir::new().unwrap();
    write_cursors(dir.path(), r#"{"C0":"1717600040.000000"}"#);
    let api = SlackApi::with_base("xoxp-test", server.uri()).unwrap();
    let rules = vec![rule(
        "general",
        "^deploy (prod|staging)$",
        true,
        "echo hit >> out.log",
        0,
    )];

    let mut store = CursorStore::load(dir.path()).unwrap();
    let channel = ChannelRef {
        id: "C0".into(),
        name: Some("general".into()),
    };
    trigger::poll_channel(&api, &mut store, &channel, &rules, Some("U_OP"), dir.path())
        .await
        .unwrap();

    let log = std::fs::read_to_string(dir.path().join("out.log")).unwrap_or_default();
    assert!(
        log.contains("hit"),
        "deploy prod should match the regex: {log}"
    );
}

#[tokio::test]
async fn ac004_regex_rejects_dev() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/auth.test"))
        .respond_with(ResponseTemplate::new(200).set_body_json(auth_test_response("U_OP")))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/conversations.list"))
        .respond_with(ResponseTemplate::new(200).set_body_json(channel_list(&[("C0", "general")])))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/conversations.history"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(history_response(&[(
                "U1",
                "deploy dev",
                "1717600046.000100",
            )])),
        )
        .mount(&server)
        .await;

    let dir = TempDir::new().unwrap();
    write_cursors(dir.path(), r#"{"C0":"1717600040.000000"}"#);
    let api = SlackApi::with_base("xoxp-test", server.uri()).unwrap();
    let rules = vec![rule(
        "general",
        "^deploy (prod|staging)$",
        true,
        "echo hit >> out.log",
        0,
    )];

    let mut store = CursorStore::load(dir.path()).unwrap();
    let channel = ChannelRef {
        id: "C0".into(),
        name: Some("general".into()),
    };
    trigger::poll_channel(&api, &mut store, &channel, &rules, Some("U_OP"), dir.path())
        .await
        .unwrap();

    let log = std::fs::read_to_string(dir.path().join("out.log")).unwrap_or_default();
    assert!(
        !log.contains("hit"),
        "deploy dev should not match the regex: {log}"
    );
}

#[tokio::test]
async fn ac006_invalid_regex_fails_at_config_load() {
    let dir = TempDir::new().unwrap();
    let path = write_config(
        dir.path(),
        r#"[{"channel":"c","message":"[unterminated","regex":true,"command":"x"}]"#,
    );
    let err = config::load_rules(&path).unwrap_err().to_string();
    assert!(err.contains("invalid rule at index 0"));
}

#[tokio::test]
async fn ac005_missing_config_exits_with_helpful_error() {
    let dir = TempDir::new().unwrap();
    let missing = dir.path().join("does-not-exist.json");
    let err = config::load_rules(&missing).unwrap_err().to_string();
    assert!(err.contains("does-not-exist.json"));
}

//! Claude plan usage for the footer: the same read-only request Claude Code's /usage makes.
//! The access token is read from Claude Code's credentials file on every call and never
//! refreshed here (refreshing would rotate Claude Code's own sign-in). It never reaches the webview.

use serde::Serialize;
use serde_json::Value;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const USAGE_URL: &str = "https://api.anthropic.com/api/oauth/usage";
const OAUTH_BETA: &str = "oauth-2025-04-20";

/// One plan limit. `kind` is "session" (5-hour window), "weekly_all", "weekly_scoped" or a newer kind.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct UsageLimit {
    pub kind: String,
    /// 0–100.
    pub percent: f64,
    /// ISO time; None until the window starts.
    pub resets_at: Option<String>,
    /// The model of a model-scoped limit, e.g. "Sonnet".
    pub model: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum UsageReport {
    Ok {
        limits: Vec<UsageLimit>,
    },
    /// No Claude sign-in on this PC.
    SignedOut,
    /// The access token ran out; Claude Code renews it on its next request.
    Expired,
    Error {
        message: String,
    },
}

fn error(message: &str) -> UsageReport {
    UsageReport::Error {
        message: message.to_string(),
    }
}

/// `%CLAUDE_CONFIG_DIR%\.credentials.json`, else `%USERPROFILE%\.claude\.credentials.json`.
fn credentials_path() -> Option<PathBuf> {
    let dir = match std::env::var_os("CLAUDE_CONFIG_DIR") {
        Some(d) if !d.is_empty() => PathBuf::from(d),
        _ => PathBuf::from(std::env::var_os("USERPROFILE")?).join(".claude"),
    };
    Some(dir.join(".credentials.json"))
}

/// The Claude sign-in's access token, or the report to show instead.
fn access_token(credentials: Option<&str>, now_ms: i64) -> Result<String, UsageReport> {
    let creds: Value = credentials
        .and_then(|t| serde_json::from_str(t).ok())
        .ok_or(UsageReport::SignedOut)?;
    let oauth = &creds["claudeAiOauth"];
    let token = oauth["accessToken"]
        .as_str()
        .filter(|t| !t.is_empty())
        .ok_or(UsageReport::SignedOut)?;
    if oauth["expiresAt"].as_i64().is_some_and(|exp| exp <= now_ms) {
        return Err(UsageReport::Expired);
    }
    Ok(token.to_string())
}

/// Reads the `limits` list; a response without it falls back to `five_hour` / `seven_day`.
fn parse_limits(body: &Value) -> Vec<UsageLimit> {
    if let Some(list) = body["limits"].as_array() {
        return list
            .iter()
            .filter_map(|l| {
                Some(UsageLimit {
                    kind: l["kind"].as_str()?.to_string(),
                    percent: l["percent"].as_f64()?,
                    resets_at: l["resets_at"].as_str().map(str::to_string),
                    model: l["scope"]["model"]["display_name"]
                        .as_str()
                        .map(str::to_string),
                })
            })
            .collect();
    }
    [("five_hour", "session"), ("seven_day", "weekly_all")]
        .iter()
        .filter_map(|(key, kind)| {
            let w = &body[*key];
            Some(UsageLimit {
                kind: kind.to_string(),
                percent: w["utilization"].as_f64()?,
                resets_at: w["resets_at"].as_str().map(str::to_string),
                model: None,
            })
        })
        .collect()
}

fn client() -> Option<&'static reqwest::Client> {
    static CLIENT: OnceLock<Option<reqwest::Client>> = OnceLock::new();
    CLIENT
        .get_or_init(|| {
            reqwest::Client::builder()
                .user_agent(concat!("claude-deck/", env!("CARGO_PKG_VERSION")))
                .timeout(Duration::from_secs(15))
                .build()
                .ok()
        })
        .as_ref()
}

pub async fn fetch() -> UsageReport {
    let text = credentials_path().and_then(|p| std::fs::read_to_string(p).ok());
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    let token = match access_token(text.as_deref(), now_ms) {
        Ok(t) => t,
        Err(report) => return report,
    };
    let Some(client) = client() else {
        return error("Cannot start the usage request");
    };
    let resp = match client
        .get(USAGE_URL)
        .bearer_auth(token)
        .header("anthropic-beta", OAUTH_BETA)
        .send()
        .await
    {
        Ok(r) => r,
        Err(_) => return error("Can't reach Anthropic to read usage"),
    };
    match resp.status().as_u16() {
        200 => {}
        401 => return UsageReport::Expired,
        429 => return error("Anthropic is limiting usage checks for now"),
        code => return error(&format!("Usage request failed (HTTP {code})")),
    }
    match resp
        .bytes()
        .await
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
    {
        Some(body) => UsageReport::Ok {
            limits: parse_limits(&body),
        },
        None => error("Anthropic sent usage data this version can't read"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn token_comes_from_the_claude_sign_in() {
        let creds = r#"{"claudeAiOauth":{"accessToken":"tok","expiresAt":2000}}"#;
        assert_eq!(access_token(Some(creds), 1000), Ok("tok".to_string()));
        assert_eq!(access_token(Some(creds), 2000), Err(UsageReport::Expired));
        let no_expiry = r#"{"claudeAiOauth":{"accessToken":"tok"}}"#;
        assert_eq!(access_token(Some(no_expiry), 1), Ok("tok".to_string()));
    }

    #[test]
    fn missing_or_unreadable_credentials_mean_signed_out() {
        assert_eq!(access_token(None, 0), Err(UsageReport::SignedOut));
        assert_eq!(
            access_token(Some("not json"), 0),
            Err(UsageReport::SignedOut)
        );
        assert_eq!(
            access_token(Some(r#"{"mcpOAuth":{}}"#), 0),
            Err(UsageReport::SignedOut)
        );
        assert_eq!(
            access_token(Some(r#"{"claudeAiOauth":{"accessToken":""}}"#), 0),
            Err(UsageReport::SignedOut)
        );
    }

    #[test]
    fn limits_list_is_read_with_the_model_of_scoped_limits() {
        let body = json!({
            "five_hour": {"utilization": 99.0, "resets_at": "ignored"},
            "limits": [
                {"kind": "session", "percent": 3, "resets_at": "2026-10-20T21:59:59Z", "scope": null},
                {"kind": "weekly_scoped", "percent": 12.5, "resets_at": null,
                 "scope": {"model": {"id": null, "display_name": "Sonnet"}}},
                {"kind": "broken"}
            ]
        });
        assert_eq!(
            parse_limits(&body),
            vec![
                UsageLimit {
                    kind: "session".into(),
                    percent: 3.0,
                    resets_at: Some("2026-10-20T21:59:59Z".into()),
                    model: None,
                },
                UsageLimit {
                    kind: "weekly_scoped".into(),
                    percent: 12.5,
                    resets_at: None,
                    model: Some("Sonnet".into()),
                },
            ]
        );
    }

    #[test]
    fn older_responses_fall_back_to_the_windows() {
        let body = json!({
            "five_hour": {"utilization": 3.0, "resets_at": "a"},
            "seven_day": {"utilization": 83.0, "resets_at": null},
            "seven_day_opus": null
        });
        let kinds: Vec<_> = parse_limits(&body)
            .into_iter()
            .map(|l| (l.kind, l.percent))
            .collect();
        assert_eq!(
            kinds,
            vec![
                ("session".to_string(), 3.0),
                ("weekly_all".to_string(), 83.0)
            ]
        );
        assert!(parse_limits(&json!({"five_hour": null})).is_empty());
    }

    #[test]
    fn report_wire_format_is_pinned() {
        let r = UsageReport::Ok {
            limits: vec![UsageLimit {
                kind: "session".into(),
                percent: 3.0,
                resets_at: None,
                model: None,
            }],
        };
        assert_eq!(
            serde_json::to_string(&r).unwrap(),
            r#"{"status":"ok","limits":[{"kind":"session","percent":3.0,"resets_at":null,"model":null}]}"#
        );
        assert_eq!(
            serde_json::to_string(&UsageReport::SignedOut).unwrap(),
            r#"{"status":"signed_out"}"#
        );
    }
}

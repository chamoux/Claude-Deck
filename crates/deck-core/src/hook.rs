//! Parses the JSON that Claude Code sends to a hook command on standard input.

use serde_json::Value;

/// What one hook call tells us about a session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HookEvent {
    SessionStart {
        source: StartSource,
    },
    UserPromptSubmit {
        prompt: String,
    },
    /// PreToolUse or PostToolUse: Claude is running a tool.
    ToolUse,
    /// A Notification asking for permission or asking the user a question.
    NeedsYou {
        message: String,
    },
    /// A Notification saying Claude has been waiting for the next prompt.
    WaitingForInput,
    /// The turn ended. `background_work` is true when the payload lists background
    /// tasks (shells, subagents) or session crons: Claude will wake itself up for
    /// them, so the session is paused, not idle.
    Stop {
        background_work: bool,
    },
    SessionEnd,
    /// Any other hook event, such as SubagentStop or StopFailure. The state
    /// machine ignores it.
    Other(String),
}

/// The `source` of a SessionStart.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StartSource {
    Startup,
    Resume,
    Clear,
    Compact,
    /// `fork`, a missing source, or one this version does not know.
    Other(String),
}

/// One parsed hook call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookCall {
    /// Claude's conversation id (the payload's session_id). It changes after /clear or /resume inside one process.
    pub conversation_id: String,
    pub event: HookEvent,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum HookParseError {
    #[error("hook input is not JSON: {0}")]
    NotJson(String),
    #[error("hook input has no {0} field")]
    MissingField(&'static str),
}

/// Parses the JSON Claude writes to a hook command's standard input.
/// A leading UTF-8 byte order mark is ignored.
pub fn parse_hook(input: &str) -> Result<HookCall, HookParseError> {
    let input = input.strip_prefix('\u{feff}').unwrap_or(input);
    let v: Value =
        serde_json::from_str(input).map_err(|e| HookParseError::NotJson(e.to_string()))?;
    let name =
        str_field(&v, "hook_event_name").ok_or(HookParseError::MissingField("hook_event_name"))?;
    let session_id = str_field(&v, "session_id")
        .ok_or(HookParseError::MissingField("session_id"))?
        .to_string();
    let event = match name {
        "SessionStart" => HookEvent::SessionStart {
            source: start_source(&v),
        },
        "UserPromptSubmit" => HookEvent::UserPromptSubmit {
            prompt: str_field(&v, "prompt").unwrap_or("").to_string(),
        },
        "PreToolUse" | "PostToolUse" => HookEvent::ToolUse,
        "Notification" => classify_notification(&v),
        "Stop" => HookEvent::Stop {
            background_work: has_background_work(&v),
        },
        "SessionEnd" => HookEvent::SessionEnd,
        other => HookEvent::Other(other.to_string()),
    };
    Ok(HookCall {
        conversation_id: session_id,
        event,
    })
}

fn start_source(v: &Value) -> StartSource {
    match str_field(v, "source").unwrap_or("") {
        "startup" => StartSource::Startup,
        "resume" => StartSource::Resume,
        "clear" => StartSource::Clear,
        "compact" => StartSource::Compact,
        other => StartSource::Other(other.to_string()),
    }
}

/// True when a Stop payload lists a background task or a session cron. Any
/// entry counts, whatever its status: a stale entry only delays auto-sleep.
fn has_background_work(v: &Value) -> bool {
    ["background_tasks", "session_crons"].iter().any(|key| {
        v.get(key)
            .and_then(Value::as_array)
            .is_some_and(|list| !list.is_empty())
    })
}

/// Notification types that need nothing from the user.
const INFO_NOTIFICATIONS: [&str; 5] = [
    "auth_success",
    "elicitation_complete",
    "elicitation_response",
    "quota_auto_resume_fired",
    "agent_completed",
];

/// "Waiting for input" only when Claude says so. Known informational types
/// change nothing. Anything else counts as "needs you", which never
/// auto-sleeps: the safe side (spec §4.2).
fn classify_notification(v: &Value) -> HookEvent {
    let message = str_field(v, "message").unwrap_or("").to_string();
    match str_field(v, "notification_type") {
        Some("idle_prompt") => HookEvent::WaitingForInput,
        Some(kind) if INFO_NOTIFICATIONS.contains(&kind) => {
            HookEvent::Other(format!("Notification:{kind}"))
        }
        Some(_) => HookEvent::NeedsYou { message },
        None if message.to_lowercase().contains("waiting for your input") => {
            HookEvent::WaitingForInput
        }
        None => HookEvent::NeedsYou { message },
    }
}

fn str_field<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(Value::as_str)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(json: &str) -> HookEvent {
        parse_hook(json).expect("valid hook json").event
    }

    #[test]
    fn reads_session_id_and_prompt() {
        let call = parse_hook(
            r#"{"session_id":"abc","hook_event_name":"UserPromptSubmit","prompt":"fix the bug"}"#,
        )
        .unwrap();
        assert_eq!(call.conversation_id, "abc");
        assert_eq!(
            call.event,
            HookEvent::UserPromptSubmit {
                prompt: "fix the bug".into()
            }
        );
    }

    #[test]
    fn tool_hooks_mean_tool_use() {
        assert_eq!(
            event(r#"{"session_id":"a","hook_event_name":"PreToolUse","tool_name":"Bash"}"#),
            HookEvent::ToolUse
        );
        assert_eq!(
            event(r#"{"session_id":"a","hook_event_name":"PostToolUse","tool_name":"Bash"}"#),
            HookEvent::ToolUse
        );
    }

    #[test]
    fn simple_events() {
        assert_eq!(
            event(r#"{"session_id":"a","hook_event_name":"Stop"}"#),
            HookEvent::Stop {
                background_work: false
            }
        );
        assert_eq!(
            event(r#"{"session_id":"a","hook_event_name":"SessionEnd","reason":"other"}"#),
            HookEvent::SessionEnd
        );
        assert_eq!(
            event(r#"{"session_id":"a","hook_event_name":"PreCompact"}"#),
            HookEvent::Other("PreCompact".into())
        );
    }

    #[test]
    fn session_start_sources() {
        let source = |s: &str| {
            event(&format!(
                r#"{{"session_id":"a","hook_event_name":"SessionStart","source":"{s}"}}"#
            ))
        };
        let start = |source| HookEvent::SessionStart { source };
        assert_eq!(source("startup"), start(StartSource::Startup));
        assert_eq!(source("resume"), start(StartSource::Resume));
        assert_eq!(source("clear"), start(StartSource::Clear));
        assert_eq!(source("compact"), start(StartSource::Compact));
        assert_eq!(source("fork"), start(StartSource::Other("fork".into())));
        assert_eq!(
            event(r#"{"session_id":"a","hook_event_name":"SessionStart"}"#),
            start(StartSource::Other(String::new()))
        );
    }

    #[test]
    fn stop_with_a_background_task_or_cron_is_background_work() {
        // Captured from Claude Code 2.1.303 on 2026-10-19 (spike/hooks.log line 36).
        let real = r#"{"session_id":"8a7a","hook_event_name":"Stop","stop_hook_active":false,
            "last_assistant_message":"The loop is running in the background.",
            "background_tasks":[{"id":"bd4s07km9","type":"shell","status":"running",
            "description":"Print numbers 1 to 30, one per second"}],"session_crons":[]}"#;
        assert_eq!(
            event(real),
            HookEvent::Stop {
                background_work: true
            }
        );
        assert_eq!(
            event(
                r#"{"session_id":"a","hook_event_name":"Stop","background_tasks":[],"session_crons":[{"id":"c1"}]}"#
            ),
            HookEvent::Stop {
                background_work: true
            }
        );
        assert_eq!(
            event(
                r#"{"session_id":"a","hook_event_name":"Stop","background_tasks":[],"session_crons":[]}"#
            ),
            HookEvent::Stop {
                background_work: false
            }
        );
    }

    #[test]
    fn subagent_stop_and_stop_failure_are_not_stop() {
        assert_eq!(
            event(r#"{"session_id":"a","hook_event_name":"SubagentStop","agent_id":"x"}"#),
            HookEvent::Other("SubagentStop".into())
        );
        assert_eq!(
            event(r#"{"session_id":"a","hook_event_name":"StopFailure","error":"rate_limit"}"#),
            HookEvent::Other("StopFailure".into())
        );
    }

    #[test]
    fn idle_prompt_notification_is_waiting_for_input() {
        assert_eq!(
            event(
                r#"{"session_id":"a","hook_event_name":"Notification","notification_type":"idle_prompt","message":"Claude is waiting for your input"}"#
            ),
            HookEvent::WaitingForInput
        );
    }

    #[test]
    fn permission_notification_needs_you() {
        // Captured from Claude Code 2.1.303 on 2026-10-19 (spike/hooks.log line 27).
        assert_eq!(
            event(
                r#"{"session_id":"a","hook_event_name":"Notification","message":"Claude needs your permission","notification_type":"permission_prompt"}"#
            ),
            HookEvent::NeedsYou {
                message: "Claude needs your permission".into()
            }
        );
    }

    #[test]
    fn notification_without_type_falls_back_to_message_text() {
        assert_eq!(
            event(
                r#"{"session_id":"a","hook_event_name":"Notification","message":"Claude is waiting for your input"}"#
            ),
            HookEvent::WaitingForInput
        );
        assert_eq!(
            event(
                r#"{"session_id":"a","hook_event_name":"Notification","message":"Claude needs your permission to use Write"}"#
            ),
            HookEvent::NeedsYou {
                message: "Claude needs your permission to use Write".into()
            }
        );
    }

    #[test]
    fn unknown_notification_type_needs_you_so_it_never_sleeps() {
        assert!(matches!(
            event(
                r#"{"session_id":"a","hook_event_name":"Notification","notification_type":"elicitation_dialog","message":"Pick one"}"#
            ),
            HookEvent::NeedsYou { .. }
        ));
        assert!(matches!(
            event(
                r#"{"session_id":"a","hook_event_name":"Notification","notification_type":"something_new","message":""}"#
            ),
            HookEvent::NeedsYou { .. }
        ));
    }

    #[test]
    fn informational_notifications_change_nothing() {
        assert_eq!(
            event(
                r#"{"session_id":"a","hook_event_name":"Notification","notification_type":"auth_success","message":"Logged in"}"#
            ),
            HookEvent::Other("Notification:auth_success".into())
        );
    }

    #[test]
    fn rejects_bad_input() {
        assert!(matches!(
            parse_hook("not json"),
            Err(HookParseError::NotJson(_))
        ));
        assert_eq!(
            parse_hook(r#"{"hook_event_name":"Stop"}"#),
            Err(HookParseError::MissingField("session_id"))
        );
        assert_eq!(
            parse_hook(r#"{"session_id":"a"}"#),
            Err(HookParseError::MissingField("hook_event_name"))
        );
    }

    #[test]
    fn a_leading_utf8_bom_is_ignored() {
        let call =
            parse_hook("\u{feff}{\"session_id\":\"a\",\"hook_event_name\":\"Stop\"}").unwrap();
        assert_eq!(
            call.event,
            HookEvent::Stop {
                background_work: false
            }
        );
    }
}

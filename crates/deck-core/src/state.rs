//! Session states and the auto-sleep rule (spec §4.2).

use crate::hook::{HookEvent, StartSource};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionState {
    Working,
    NeedsYou,
    Idle,
    Asleep,
}

/// Live status of one session. Times are Unix milliseconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionStatus {
    pub state: SessionState,
    /// While Idle: when the idle clock started.
    pub idle_since_ms: Option<i64>,
    /// The last Stop listed background tasks or session crons. Until a later
    /// Stop lists none, nothing makes this session Idle: sleeping it would kill
    /// that work.
    pub background_work: bool,
}

impl SessionStatus {
    /// A Claude process that was just started. It counts as Working until a
    /// hook says otherwise, so it can never be auto-slept by mistake.
    pub fn started() -> Self {
        Self {
            state: SessionState::Working,
            idle_since_ms: None,
            background_work: false,
        }
    }

    pub fn asleep() -> Self {
        Self {
            state: SessionState::Asleep,
            idle_since_ms: None,
            background_work: false,
        }
    }

    /// Applies one hook event. Hooks that arrive after the process exited are ignored.
    pub fn apply(&mut self, event: &HookEvent, now_ms: i64) {
        if self.state == SessionState::Asleep {
            return;
        }
        match event {
            HookEvent::SessionStart {
                source: StartSource::Startup | StartSource::Resume,
            } => self.idle_unless_background(now_ms),
            // Auto-compaction can run in the middle of a turn, and /clear or an
            // unknown source says nothing about whether Claude is busy.
            HookEvent::SessionStart { .. } => {}
            HookEvent::Stop { background_work } => {
                self.background_work = *background_work;
                self.idle_unless_background(now_ms);
            }
            HookEvent::UserPromptSubmit { .. } | HookEvent::ToolUse => {
                self.set(SessionState::Working)
            }
            HookEvent::NeedsYou { .. } => self.set(SessionState::NeedsYou),
            // Claude says it has waited ~60 s for the next prompt. That is the only
            // signal after Esc on a permission prompt, which sends no hook (spike 2,
            // 2026-10-20), so it ends Working. Never with background work (sleeping
            // would kill it), never from Needs you (an unanswered question stays),
            // and an existing idle clock is kept.
            HookEvent::WaitingForInput => {
                if self.state == SessionState::Working && !self.background_work {
                    self.idle_unless_background(now_ms);
                }
            }
            // /clear and /resume end the conversation but keep the process.
            // Only on_exit, the process really ending, means Asleep.
            HookEvent::SessionEnd => {}
            HookEvent::Other(_) => {}
        }
    }

    /// Keyboard input: restarts an Idle session's idle clock, and moves a Needs you session to Working (the user answered).
    pub fn on_input(&mut self, now_ms: i64) {
        match self.state {
            SessionState::Idle => self.idle_since_ms = Some(now_ms),
            SessionState::NeedsYou => {
                self.state = SessionState::Working;
                self.idle_since_ms = None;
            }
            _ => {}
        }
    }

    /// The Claude process ended: by itself, by sleeping, or by a crash.
    pub fn on_exit(&mut self) {
        *self = Self::asleep();
    }

    /// True when an Idle session has been idle for `auto_sleep_minutes`.
    /// `None` means auto-sleep is off.
    pub fn should_auto_sleep(&self, now_ms: i64, auto_sleep_minutes: Option<u32>) -> bool {
        match (self.state, self.idle_since_ms, auto_sleep_minutes) {
            (SessionState::Idle, Some(since), Some(minutes)) if !self.background_work => {
                now_ms - since >= i64::from(minutes) * 60_000
            }
            _ => false,
        }
    }

    /// "Sleep now" may skip the confirmation only for Idle sessions.
    pub fn can_sleep_without_asking(&self) -> bool {
        self.state == SessionState::Idle && !self.background_work
    }

    /// Working or Needs you: closing the window shows the dialog (spec §4.4).
    pub fn is_busy(&self) -> bool {
        matches!(self.state, SessionState::Working | SessionState::NeedsYou)
    }

    fn idle_unless_background(&mut self, now_ms: i64) {
        if self.background_work {
            self.set(SessionState::Working);
        } else {
            self.state = SessionState::Idle;
            self.idle_since_ms = Some(now_ms);
        }
    }

    fn set(&mut self, state: SessionState) {
        self.state = state;
        self.idle_since_ms = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hook::{HookEvent, StartSource};

    const MIN: i64 = 60_000;
    const T0: i64 = 1_000_000;
    const STOP: HookEvent = HookEvent::Stop {
        background_work: false,
    };
    const STOP_WITH_BACKGROUND: HookEvent = HookEvent::Stop {
        background_work: true,
    };

    fn start(source: StartSource) -> HookEvent {
        HookEvent::SessionStart { source }
    }

    fn prompt(text: &str) -> HookEvent {
        HookEvent::UserPromptSubmit {
            prompt: text.into(),
        }
    }

    fn idle(since: i64) -> SessionStatus {
        SessionStatus {
            state: SessionState::Idle,
            idle_since_ms: Some(since),
            background_work: false,
        }
    }

    fn idle_at(t: i64) -> SessionStatus {
        let mut s = SessionStatus::started();
        s.apply(&STOP, t);
        s
    }

    #[test]
    fn a_new_process_counts_as_working_and_never_auto_sleeps() {
        let s = SessionStatus::started();
        assert_eq!(s.state, SessionState::Working);
        assert!(!s.should_auto_sleep(T0 + 600 * MIN, Some(30)));
    }

    #[test]
    fn session_start_prompt_and_stop() {
        let mut s = SessionStatus::started();
        s.apply(&start(StartSource::Startup), T0);
        assert_eq!(s, idle(T0));
        s.apply(&prompt("go"), T0 + 1);
        assert_eq!(s, SessionStatus::started());
        s.apply(&STOP, T0 + 2);
        assert_eq!(s, idle(T0 + 2));
    }

    #[test]
    fn waking_with_resume_is_idle() {
        let mut s = SessionStatus::started();
        s.apply(&start(StartSource::Resume), T0);
        assert_eq!(s, idle(T0));
    }

    #[test]
    fn compact_clear_and_unknown_starts_keep_the_state() {
        // Auto-compaction mid-turn must not make a working session Idle.
        let mut w = SessionStatus::started();
        w.apply(&prompt("refactor"), T0);
        for source in [
            StartSource::Compact,
            StartSource::Clear,
            StartSource::Other("fork".into()),
        ] {
            w.apply(&start(source), T0 + MIN);
            assert_eq!(w, SessionStatus::started());
        }
        // And an Idle session keeps its clock.
        let mut s = idle_at(T0);
        s.apply(&start(StartSource::Compact), T0 + 5 * MIN);
        assert_eq!(s, idle(T0));
    }

    #[test]
    fn a_stop_with_background_work_keeps_the_session_working_until_a_clean_stop() {
        // The spike's background loop: prompt, Bash in background, Stop listing it.
        let mut s = SessionStatus::started();
        s.apply(&prompt("run a loop in the background"), T0);
        s.apply(&HookEvent::ToolUse, T0 + 1);
        s.apply(&STOP_WITH_BACKGROUND, T0 + 2);
        assert_eq!(s.state, SessionState::Working);
        assert!(s.is_busy());
        assert!(!s.can_sleep_without_asking());
        // 60 s later Claude says it waits for input: that must not end Working.
        s.apply(&HookEvent::WaitingForInput, T0 + MIN);
        assert_eq!(s.state, SessionState::Working);
        assert!(!s.should_auto_sleep(T0 + 600 * MIN, Some(30)));
        // The task ends: Claude injects a <task-notification> prompt, then stops clean.
        s.apply(
            &prompt("<task-notification>...</task-notification>"),
            T0 + 30 * MIN,
        );
        assert!(s.background_work);
        s.apply(&STOP, T0 + 31 * MIN);
        assert_eq!(s, idle(T0 + 31 * MIN));
    }

    #[test]
    fn only_a_stop_ends_background_work() {
        let mut s = SessionStatus::started();
        s.apply(&STOP_WITH_BACKGROUND, T0);
        for event in [
            start(StartSource::Resume),
            start(StartSource::Clear),
            HookEvent::WaitingForInput,
            HookEvent::SessionEnd,
            HookEvent::Other("SubagentStop".into()),
            HookEvent::NeedsYou {
                message: "permission".into(),
            },
            HookEvent::ToolUse,
        ] {
            s.apply(&event, T0 + 1);
            assert!(
                s.is_busy(),
                "{event:?} made a session with background work idle"
            );
            assert!(s.background_work);
        }
    }

    #[test]
    fn auto_sleep_never_fires_while_background_work_is_known() {
        let s = SessionStatus {
            background_work: true,
            ..idle(T0)
        };
        assert!(!s.should_auto_sleep(T0 + 600 * MIN, Some(30)));
        assert!(!s.can_sleep_without_asking());
    }

    #[test]
    fn idle_sleeps_after_the_set_minutes() {
        let s = idle_at(T0);
        assert!(!s.should_auto_sleep(T0 + 30 * MIN - 1, Some(30)));
        assert!(s.should_auto_sleep(T0 + 30 * MIN, Some(30)));
    }

    #[test]
    fn auto_sleep_off_never_sleeps() {
        assert!(!idle_at(T0).should_auto_sleep(T0 + 10_000 * MIN, None));
    }

    #[test]
    fn needs_you_never_sleeps_and_tool_use_clears_it() {
        let mut s = SessionStatus::started();
        s.apply(
            &HookEvent::NeedsYou {
                message: "permission".into(),
            },
            T0,
        );
        assert_eq!(s.state, SessionState::NeedsYou);
        assert!(!s.should_auto_sleep(T0 + 10_000 * MIN, Some(30)));
        s.apply(&HookEvent::ToolUse, T0 + 1);
        assert_eq!(s.state, SessionState::Working);
    }

    #[test]
    fn waiting_for_input_ends_working_without_background_work() {
        // Spike 2 (2026-10-20): Esc on a permission prompt sends no hook. The
        // keypress moves Needs you to Working (on_input), and idle_prompt about
        // 60 s later is the only signal that Claude is waiting again.
        let mut s = SessionStatus::started();
        s.apply(&prompt("count"), T0);
        s.apply(&HookEvent::WaitingForInput, T0 + MIN);
        assert_eq!(s, idle(T0 + MIN));
    }

    #[test]
    fn esc_on_a_permission_prompt_then_idle_prompt_ends_idle() {
        let mut s = SessionStatus::started();
        s.apply(&prompt("create x.txt"), T0);
        s.apply(
            &HookEvent::NeedsYou {
                message: "permission".into(),
            },
            T0 + 1,
        );
        s.on_input(T0 + 2); // Esc
        assert_eq!(s.state, SessionState::Working);
        s.apply(&HookEvent::WaitingForInput, T0 + MIN);
        assert_eq!(s, idle(T0 + MIN));
    }

    #[test]
    fn waiting_for_input_keeps_an_existing_idle_clock() {
        let mut s = idle_at(T0);
        s.apply(&HookEvent::WaitingForInput, T0 + MIN);
        assert_eq!(s, idle(T0));
    }

    #[test]
    fn waiting_for_input_does_not_clear_needs_you() {
        let mut s = SessionStatus::started();
        s.apply(
            &HookEvent::NeedsYou {
                message: "permission".into(),
            },
            T0,
        );
        s.apply(&HookEvent::WaitingForInput, T0 + MIN);
        assert_eq!(s.state, SessionState::NeedsYou);
    }

    #[test]
    fn typing_answers_a_needs_you_session() {
        let mut s = SessionStatus::started();
        s.apply(
            &HookEvent::NeedsYou {
                message: "permission".into(),
            },
            T0,
        );
        s.on_input(T0 + 5);
        assert_eq!(s.state, SessionState::Working);
        assert_eq!(s.idle_since_ms, None);
    }

    #[test]
    fn typing_restarts_the_idle_clock_only_when_idle() {
        let mut s = idle_at(T0);
        s.on_input(T0 + 20 * MIN);
        assert_eq!(s.idle_since_ms, Some(T0 + 20 * MIN));
        assert!(!s.should_auto_sleep(T0 + 30 * MIN, Some(30)));

        let mut w = SessionStatus::started();
        w.on_input(T0);
        assert_eq!(w, SessionStatus::started());
    }

    #[test]
    fn session_end_keeps_tracking_because_clear_and_resume_keep_the_process() {
        // /clear sends SessionEnd (reason "clear"), then SessionStart, and Claude runs on.
        let mut s = idle_at(T0);
        s.apply(&HookEvent::SessionEnd, T0 + 1);
        s.apply(&start(StartSource::Clear), T0 + 2);
        assert_eq!(s, idle(T0));
        s.apply(&prompt("a big new task"), T0 + 3);
        assert_eq!(s.state, SessionState::Working);
    }

    #[test]
    fn exit_means_asleep_and_later_hooks_are_ignored() {
        let mut s = idle_at(T0);
        s.on_exit();
        assert_eq!(s, SessionStatus::asleep());
        s.apply(&prompt("late"), T0 + 2);
        assert_eq!(s, SessionStatus::asleep());
    }

    #[test]
    fn other_events_change_nothing() {
        // SubagentStop is not the end of the turn, and StopFailure may be a usage
        // limit that Claude resumes by itself later: both leave Working alone.
        let mut w = SessionStatus::started();
        for name in [
            "SubagentStop",
            "StopFailure",
            "PreCompact",
            "Notification:auth_success",
        ] {
            w.apply(&HookEvent::Other(name.into()), T0);
            assert_eq!(w, SessionStatus::started());
        }
        let mut s = idle_at(T0);
        s.apply(&HookEvent::Other("PreCompact".into()), T0 + 5);
        assert_eq!(s, idle_at(T0));
    }

    #[test]
    fn busy_and_sleep_without_asking() {
        assert!(SessionStatus::started().is_busy());
        assert!(!SessionStatus::started().can_sleep_without_asking());
        assert!(!idle_at(T0).is_busy());
        assert!(idle_at(T0).can_sleep_without_asking());
        assert!(!SessionStatus::asleep().is_busy());
    }
}

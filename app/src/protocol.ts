// Mirrors crates/deck-proto/src/messages.rs and the deck-core types it uses (serde JSON).

import type { LayoutNode } from './layout';

export type SessionState = 'working' | 'needs_you' | 'idle' | 'asleep';
export type ChangeKind = 'created' | 'modified' | 'deleted' | 'renamed';
/** serde snake_case: PowerShell -> "power_shell", GitBash -> "git_bash". */
export type ShellKind = 'power_shell' | 'git_bash' | 'cmd';
export type ClientKind = 'window' | 'hook';

export interface Change {
  kind: ChangeKind;
  path: string;
  old_path: string | null;
  conflict: boolean;
}
export interface ChangeList {
  items: Change[];
  total: number;
}
export interface ShellPanel {
  open: boolean;
  width: number;
}
export interface Project {
  id: string;
  path: string;
  name: string;
  shell_panel: ShellPanel;
  /** Saved split layout of the project view. Absent or null in old data. */
  layout?: LayoutNode | null;
}
/** A named, saved combination of sessions (may span projects). `layout` is `{}` for a new group; run it through `normalize`. */
export interface Group {
  id: string;
  name: string;
  layout: LayoutNode | Record<string, never>;
}
export interface WindowPlacement {
  x: number;
  y: number;
  width: number;
  height: number;
}
export interface Settings {
  auto_sleep_minutes: number | null;
  claude_path: string | null;
  window: WindowPlacement | null;
  shell: ShellKind;
}
export interface PastSession {
  id: string;
  last_prompt: string | null;
  modified_ms: number;
}

export type Target =
  | { kind: 'session'; deck_id: string }
  | { kind: 'shell'; project_id: string };

export type Request =
  | { type: 'hello'; client: ClientKind }
  | { type: 'get_state' }
  | { type: 'add_project'; path: string }
  | { type: 'remove_project'; project_id: string; force: boolean }
  | { type: 'new_session'; project_id: string }
  /** Moves a project to `index` in the sidebar order (clamped to the end). */
  | { type: 'move_project'; project_id: string; index: number }
  | { type: 'wake'; deck_id: string }
  | { type: 'sleep'; deck_id: string; force: boolean }
  | { type: 'close_session'; deck_id: string; force: boolean }
  | { type: 'rename'; deck_id: string; name: string }
  | { type: 'attach'; target: Target; rows: number; cols: number }
  | { type: 'detach'; target: Target }
  | { type: 'resize'; target: Target; rows: number; cols: number }
  | { type: 'close_shell'; project_id: string }
  | { type: 'set_shell_panel'; project_id: string; panel: ShellPanel }
  | { type: 'git_status'; project_id: string }
  | { type: 'list_past_sessions'; project_id: string }
  | { type: 'open_past_session'; project_id: string; conversation_id: string }
  | { type: 'set_settings'; settings: Settings }
  | { type: 'window_closing'; keep_running: boolean }
  | { type: 'hook_event'; deck_id: string; payload: string }
  | { type: 'set_project_layout'; project_id: string; layout: LayoutNode | null }
  | { type: 'add_group'; name: string }
  | { type: 'rename_group'; group_id: string; name: string }
  | { type: 'remove_group'; group_id: string }
  | { type: 'set_group_layout'; group_id: string; layout: LayoutNode };

/** The request to resend, with `force: true`, after the user confirms a `needs_confirm` event. */
export type ConfirmAction =
  | { type: 'sleep'; deck_id: string }
  | { type: 'close_session'; deck_id: string }
  | { type: 'remove_project'; project_id: string };

export interface SessionView {
  deck_id: string;
  project_id: string;
  name: string | null;
  state: SessionState;
  background_work: boolean;
  /** e.g. "Can't resume" or "Claude quit: the folder was not trusted". */
  error: string | null;
}
export interface ShellView {
  project_id: string;
  running: boolean;
}
export interface StateView {
  projects: Project[];
  sessions: SessionView[];
  shells: ShellView[];
  settings: Settings;
  missing_folders: string[];
  /** Absent in data from an older helper; treat as empty. */
  groups?: Group[];
}

export type GitResult =
  | { status: 'ok'; list: ChangeList }
  | { status: 'not_git' }
  | { status: 'git_missing' }
  | { status: 'error'; message: string };

export type Event =
  | { type: 'state'; state: StateView }
  | { type: 'session_changed'; session: SessionView }
  | { type: 'shell_changed'; shell: ShellView }
  | { type: 'attached'; target: Target; index: number }
  | { type: 'detached'; target: Target }
  | { type: 'memory'; awake: number; bytes: number }
  | { type: 'git_status'; project_id: string; result: GitResult }
  | { type: 'past_sessions'; project_id: string; sessions: PastSession[] }
  | { type: 'needs_confirm'; action: ConfirmAction; message: string }
  | { type: 'error'; message: string };

export function targetKey(t: Target): string {
  return t.kind === 'session' ? `session:${t.deck_id}` : `shell:${t.project_id}`;
}

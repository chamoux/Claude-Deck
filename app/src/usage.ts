// Claude plan usage for the footer. Mirrors app/src-tauri/src/usage.rs (serde JSON).

import { ref } from 'vue';
import type { Ref } from 'vue';

/** One plan limit. `kind` is 'session' (5-hour window), 'weekly_all', 'weekly_scoped' or a newer kind. */
export interface UsageLimit {
  kind: string;
  /** 0–100. */
  percent: number;
  /** ISO time; null until the window starts. */
  resets_at: string | null;
  /** The model of a model-scoped limit, e.g. "Sonnet". */
  model: string | null;
}

export type UsageReport =
  | { status: 'ok'; limits: UsageLimit[] }
  | { status: 'signed_out' }
  | { status: 'expired' }
  | { status: 'error'; message: string };

export type UsageProblem = { kind: 'signed_out' | 'expired' | 'error'; message: string };
export type UsageLevel = 'normal' | 'warning' | 'critical';

/** How often usage is fetched while the window is visible. */
export const USAGE_POLL_MS = 60_000;
/** Wait after a failed fetch, so a limited or offline endpoint is not hammered. */
export const USAGE_RETRY_MS = 5 * 60_000;
/** Clock tick for countdowns and for checking whether a fetch is due. */
export const USAGE_TICK_MS = 15_000;

export interface Usage {
  /** Last good limits, kept while later fetches fail; null before the first success. */
  limits: Ref<UsageLimit[] | null>;
  fetchedAt: Ref<number | null>;
  /** Why the last fetch gave no data, or null. */
  problem: Ref<UsageProblem | null>;
  loading: Ref<boolean>;
  /** Wall clock, ticking every USAGE_TICK_MS, for countdowns. */
  now: Ref<number>;
  /** Fetches now (the panel's refresh button); a fetch already running is not doubled. */
  refresh(): Promise<void>;
  start(): void;
  stop(): void;
}

function problemOf(r: Exclude<UsageReport, { status: 'ok' }>): UsageProblem {
  switch (r.status) {
    case 'signed_out':
      return { kind: 'signed_out', message: 'Sign in to Claude Code to see your usage.' };
    case 'expired':
      return { kind: 'expired', message: 'Claude sign-in is renewing. Usage comes back after a session talks to Claude.' };
    case 'error':
      return { kind: 'error', message: r.message };
  }
}

export function createUsage(fetchReport: () => Promise<UsageReport>, clock: () => number = Date.now): Usage {
  const limits = ref<UsageLimit[] | null>(null);
  const fetchedAt = ref<number | null>(null);
  const problem = ref<UsageProblem | null>(null);
  const loading = ref(false);
  const now = ref(clock());
  let nextAt = 0;
  let timer: ReturnType<typeof setInterval> | undefined;

  async function refresh(): Promise<void> {
    if (loading.value) return;
    loading.value = true;
    let r: UsageReport;
    try {
      r = await fetchReport();
    } catch (e) {
      r = { status: 'error', message: `Usage unavailable: ${String(e)}` };
    }
    loading.value = false;
    now.value = clock();
    if (r.status === 'ok') {
      limits.value = r.limits;
      fetchedAt.value = now.value;
      problem.value = null;
      nextAt = now.value + USAGE_POLL_MS;
    } else {
      problem.value = problemOf(r);
      nextAt = now.value + USAGE_RETRY_MS;
    }
  }

  // Polls only while the window is visible; coming back shows fresh numbers if a fetch is due.
  function tick(): void {
    now.value = clock();
    if (!document.hidden && now.value >= nextAt) void refresh();
  }

  function stop(): void {
    if (timer !== undefined) clearInterval(timer);
    timer = undefined;
    document.removeEventListener('visibilitychange', tick);
  }

  function start(): void {
    stop();
    nextAt = 0;
    tick();
    timer = setInterval(tick, USAGE_TICK_MS);
    document.addEventListener('visibilitychange', tick);
  }

  return { limits, fetchedAt, problem, loading, now, refresh, start, stop };
}

/** "Session", "Week", "Week · Sonnet" (short, the footer) or the /usage wording (long, the panel). */
export function usageLabel(l: UsageLimit, short = false): string {
  switch (l.kind) {
    case 'session':
      return short ? 'Session' : 'Current session';
    case 'weekly_all':
      return short ? 'Week' : 'Current week (all models)';
    case 'weekly_scoped':
      return short ? `Week · ${l.model ?? 'model'}` : `Current week (${l.model ?? 'one model'})`;
    default:
      return l.kind.replace(/_/g, ' ');
  }
}

export function usageLevel(percent: number): UsageLevel {
  if (percent >= 90) return 'critical';
  if (percent >= 75) return 'warning';
  return 'normal';
}

/** The limits the footer shows: the session and the all-models week, else the first two. */
export function footerLimits(all: UsageLimit[]): UsageLimit[] {
  const picked = ['session', 'weekly_all'].flatMap((k) => all.filter((l) => l.kind === k).slice(0, 1));
  return picked.length > 0 ? picked : all.slice(0, 2);
}

/** "42m", "3h 5m", "2d 4h"; rounds up so a countdown never shows 0 before the reset. */
export function formatDuration(ms: number): string {
  if (ms <= 0) return 'now';
  const minutes = Math.ceil(ms / 60_000);
  if (minutes < 60) return `${minutes}m`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours}h ${minutes % 60}m`;
  return `${Math.floor(hours / 24)}d ${hours % 24}h`;
}

/** Local reset time: "23:59" today, else "Fri 15:59". */
export function formatResetAt(iso: string, now: number): string {
  const at = new Date(iso);
  const time = at.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
  if (at.toDateString() === new Date(now).toDateString()) return time;
  return `${at.toLocaleDateString([], { weekday: 'short' })} ${time}`;
}

export function formatAgo(ms: number): string {
  if (ms < 60_000) return 'just now';
  const minutes = Math.floor(ms / 60_000);
  if (minutes < 60) return `${minutes} min ago`;
  return `${Math.floor(minutes / 60)} h ago`;
}

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import {
  createUsage,
  footerLimits,
  formatAgo,
  formatDuration,
  usageLabel,
  usageLevel,
  USAGE_POLL_MS,
  USAGE_RETRY_MS,
  USAGE_TICK_MS,
} from './usage';
import type { UsageLimit, UsageReport } from './usage';

const lim = (kind: string, percent: number, model: string | null = null): UsageLimit => ({
  kind,
  percent,
  resets_at: null,
  model,
});
const ok = (...limits: UsageLimit[]): UsageReport => ({ status: 'ok', limits });

let hidden = false;
beforeEach(() => {
  vi.useFakeTimers();
  hidden = false;
  Object.defineProperty(document, 'hidden', { configurable: true, get: () => hidden });
});
afterEach(() => {
  vi.useRealTimers();
});

function setup(first: UsageReport = ok(lim('session', 3))) {
  let next = first;
  const fetch = vi.fn(() => Promise.resolve(next));
  const usage = createUsage(fetch);
  return { usage, fetch, answer: (r: UsageReport) => (next = r) };
}

describe('createUsage', () => {
  it('fetches at start, then every minute', async () => {
    const { usage, fetch } = setup();
    usage.start();
    await vi.advanceTimersByTimeAsync(0);
    expect(fetch).toHaveBeenCalledTimes(1);
    expect(usage.limits.value).toEqual([lim('session', 3)]);
    expect(usage.fetchedAt.value).toBe(Date.now());
    await vi.advanceTimersByTimeAsync(USAGE_POLL_MS - USAGE_TICK_MS);
    expect(fetch).toHaveBeenCalledTimes(1);
    await vi.advanceTimersByTimeAsync(USAGE_TICK_MS);
    expect(fetch).toHaveBeenCalledTimes(2);
    usage.stop();
    await vi.advanceTimersByTimeAsync(USAGE_POLL_MS * 3);
    expect(fetch).toHaveBeenCalledTimes(2);
  });

  it('keeps the last numbers when a fetch fails, says why, and waits longer', async () => {
    const { usage, fetch, answer } = setup();
    usage.start();
    await vi.advanceTimersByTimeAsync(0);
    answer({ status: 'error', message: 'offline' });
    await vi.advanceTimersByTimeAsync(USAGE_POLL_MS);
    expect(usage.limits.value).toEqual([lim('session', 3)]);
    expect(usage.problem.value).toEqual({ kind: 'error', message: 'offline' });
    const calls = fetch.mock.calls.length;
    await vi.advanceTimersByTimeAsync(USAGE_RETRY_MS - USAGE_TICK_MS);
    expect(fetch).toHaveBeenCalledTimes(calls);
    answer(ok(lim('session', 9)));
    await vi.advanceTimersByTimeAsync(USAGE_TICK_MS);
    expect(usage.limits.value).toEqual([lim('session', 9)]);
    expect(usage.problem.value).toBeNull();
    usage.stop();
  });

  it('explains a signed-out or renewing sign-in, and a thrown error', async () => {
    const { usage } = setup({ status: 'signed_out' });
    await usage.refresh();
    expect(usage.problem.value?.kind).toBe('signed_out');
    expect(usage.limits.value).toBeNull();
    const expired = createUsage(() => Promise.resolve({ status: 'expired' }));
    await expired.refresh();
    expect(expired.problem.value?.message).toContain('renewing');
    const broken = createUsage(() => Promise.reject(new Error('no tauri')));
    await broken.refresh();
    expect(broken.problem.value).toEqual({ kind: 'error', message: 'Usage unavailable: Error: no tauri' });
    expect(broken.loading.value).toBe(false);
  });

  it('does not poll while the window is hidden, and catches up when shown', async () => {
    const { usage, fetch } = setup();
    usage.start();
    await vi.advanceTimersByTimeAsync(0);
    hidden = true;
    await vi.advanceTimersByTimeAsync(USAGE_POLL_MS * 5);
    expect(fetch).toHaveBeenCalledTimes(1);
    hidden = false;
    document.dispatchEvent(new Event('visibilitychange'));
    await vi.advanceTimersByTimeAsync(0);
    expect(fetch).toHaveBeenCalledTimes(2);
    usage.stop();
  });

  it('a refresh while one is running is not doubled', async () => {
    let resolve: (r: UsageReport) => void = () => {};
    const fetch = vi.fn(() => new Promise<UsageReport>((r) => (resolve = r)));
    const usage = createUsage(fetch);
    const a = usage.refresh();
    void usage.refresh();
    expect(usage.loading.value).toBe(true);
    resolve(ok());
    await a;
    expect(fetch).toHaveBeenCalledTimes(1);
    expect(usage.loading.value).toBe(false);
  });
});

describe('usage formatting', () => {
  it('labels limits short for the footer and long for the panel', () => {
    expect(usageLabel(lim('session', 0), true)).toBe('Session');
    expect(usageLabel(lim('weekly_all', 0))).toBe('Current week (all models)');
    expect(usageLabel(lim('weekly_scoped', 0, 'Sonnet'), true)).toBe('Week · Sonnet');
    expect(usageLabel(lim('weekly_scoped', 0, 'Sonnet'))).toBe('Current week (Sonnet)');
    expect(usageLabel(lim('monthly_extra', 0))).toBe('monthly extra');
  });

  it('turns amber at 75% and red at 90%', () => {
    expect([0, 74.9, 75, 89.9, 90, 100].map(usageLevel)).toEqual([
      'normal',
      'normal',
      'warning',
      'warning',
      'critical',
      'critical',
    ]);
  });

  it('shows the session and the all-models week in the footer', () => {
    const all = [lim('weekly_scoped', 1, 'Sonnet'), lim('weekly_all', 2), lim('session', 3)];
    expect(footerLimits(all).map((l) => l.kind)).toEqual(['session', 'weekly_all']);
    expect(footerLimits([lim('a', 1), lim('b', 2), lim('c', 3)]).map((l) => l.kind)).toEqual(['a', 'b']);
  });

  it('formats countdowns, rounding up', () => {
    const m = 60_000;
    expect(formatDuration(0)).toBe('now');
    expect(formatDuration(1)).toBe('1m');
    expect(formatDuration(42 * m)).toBe('42m');
    expect(formatDuration(60 * m)).toBe('1h 0m');
    expect(formatDuration(134 * m)).toBe('2h 14m');
    expect(formatDuration((3 * 24 + 17) * 60 * m)).toBe('3d 17h');
  });

  it('formats how long ago the numbers were fetched', () => {
    expect(formatAgo(59_000)).toBe('just now');
    expect(formatAgo(3 * 60_000)).toBe('3 min ago');
    expect(formatAgo(2 * 3600_000)).toBe('2 h ago');
  });
});

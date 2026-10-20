import { flushPromises, mount } from '@vue/test-utils';
import { beforeEach, describe, expect, it } from 'vitest';
import { defineComponent, h } from 'vue';
import { fakeTransport } from '../transport';
import type { UsageReport } from '../usage';
import { createDeck, provideDeck } from '../useDeck';
import UsageMeters from './UsageMeters.vue';

const HOUR = 3600_000;

async function setup(report: UsageReport) {
  const t = fakeTransport();
  t.usageReport = report;
  const deck = createDeck(t);
  const Host = defineComponent({
    setup() {
      provideDeck(deck);
      return () => h(UsageMeters);
    },
  });
  const w = mount(Host, { attachTo: document.body });
  await deck.usage.refresh();
  await flushPromises();
  return { t, deck, w };
}

const report = (): UsageReport => ({
  status: 'ok',
  limits: [
    { kind: 'weekly_scoped', percent: 12, resets_at: null, model: 'Sonnet' },
    { kind: 'session', percent: 42.4, resets_at: new Date(Date.now() + 2 * HOUR + 14 * 60_000).toISOString(), model: null },
    { kind: 'weekly_all', percent: 91, resets_at: new Date(Date.now() + 80 * HOUR).toISOString(), model: null },
  ],
});

beforeEach(() => {
  document.body.innerHTML = '';
});

describe('UsageMeters', () => {
  it('shows the session and the week in the footer with percent, bar and countdown', async () => {
    const { w } = await setup(report());
    const meters = w.findAll('[data-testid="usage-meter"]');
    expect(meters.map((m) => m.text())).toEqual(['Session42%2h 14m', 'Week91%3d 8h']);
    const fills = w.findAll('[data-testid="usage-fill"]');
    expect((fills[0].element as HTMLElement).style.width).toBe('42.4%');
    expect(fills[0].classes()).toContain('bg-deck-accent');
    expect(fills[1].classes()).toContain('bg-deck-danger');
    expect(w.get('[data-testid="usage-toggle"]').attributes('aria-label')).toContain('Current session 42% used, resets in 2h 14m');
  });

  it('opens a panel with every limit; Escape closes it', async () => {
    const { w } = await setup(report());
    await w.get('[data-testid="usage-toggle"]').trigger('click');
    const rows = w.findAll('[data-testid="usage-row"]');
    expect(rows.map((r) => r.find('span').text())).toEqual([
      'Current week (Sonnet)',
      'Current session',
      'Current week (all models)',
    ]);
    expect(rows[1].get('[role="meter"]').attributes('aria-valuenow')).toBe('42');
    expect(rows[1].text()).toContain('Resets in 2h 14m');
    expect(w.get('[data-testid="usage-updated"]').text()).toContain('Updated just now');
    document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' }));
    await flushPromises();
    expect(w.find('[data-testid="usage-panel"]').exists()).toBe(false);
  });

  it('the refresh button fetches again; a click outside closes the panel', async () => {
    const { w, t } = await setup(report());
    await w.get('[data-testid="usage-toggle"]').trigger('click');
    const before = t.usageCalls;
    await w.get('[data-testid="usage-refresh"]').trigger('click');
    await flushPromises();
    expect(t.usageCalls).toBe(before + 1);
    document.body.dispatchEvent(new Event('pointerdown', { bubbles: true }));
    await flushPromises();
    expect(w.find('[data-testid="usage-panel"]').exists()).toBe(false);
  });

  it('says why there are no numbers', async () => {
    const { w } = await setup({ status: 'signed_out' });
    expect(w.get('[data-testid="usage-empty"]').text()).toBe('Usage unavailable');
    await w.get('[data-testid="usage-toggle"]').trigger('click');
    expect(w.get('[data-testid="usage-problem"]').text()).toContain('Sign in to Claude Code');
  });

  it('dims stale numbers after a failed fetch', async () => {
    const { w, t, deck } = await setup(report());
    t.usageReport = { status: 'error', message: 'Usage request failed (HTTP 500)' };
    await deck.usage.refresh();
    await flushPromises();
    expect(w.get('[data-testid="usage-meter"]').classes()).toContain('opacity-60');
    expect(w.get('[data-testid="usage-toggle"]').attributes('title')).toBe('Usage request failed (HTTP 500)');
  });
});

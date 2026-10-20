import { mount } from '@vue/test-utils';
import { describe, expect, it, vi } from 'vitest';
import type { SessionState, SessionView } from '../protocol';
import SessionRow from './SessionRow.vue';

function sess(state: SessionState, extra: Partial<SessionView> = {}): SessionView {
  return { deck_id: 'd1', project_id: 'p1', name: 'Fix bug', state, background_work: false, error: null, ...extra };
}
const mk = (s: SessionView, props: Record<string, unknown> = {}) =>
  mount(SessionRow, { props: { session: s, active: false, renaming: false, ...props } });

describe('SessionRow', () => {
  it.each([
    ['working', 'bg-dot-working'],
    ['idle', 'bg-dot-idle'],
    ['needs_you', 'bg-dot-needs'],
    ['asleep', 'border-dot-asleep'],
  ] as const)('%s dot uses %s', (state, cls) => {
    expect(mk(sess(state)).get('[data-testid="dot"]').classes()).toContain(cls);
  });

  it('working dot pulses and respects reduced motion', () => {
    const c = mk(sess('working')).get('[data-testid="dot"]').classes();
    expect(c).toContain('animate-pulse');
    expect(c).toContain('motion-reduce:animate-none');
  });

  it('falls back to New session and sets title', () => {
    const w = mk(sess('idle', { name: null }));
    expect(w.text()).toContain('New session');
    const w2 = mk(sess('idle', { name: 'A very long name' }));
    expect(w2.get('[data-testid="name"]').attributes('title')).toBe('A very long name');
  });

  it('needs_you shows label and tint', () => {
    const w = mk(sess('needs_you'));
    expect(w.text()).toContain('Needs you');
    expect(w.get('[data-nav]').classes()).toContain('bg-needs-soft');
  });

  it('shows the resume error', () => {
    expect(mk(sess('asleep', { error: 'boom' })).text()).toContain("Can't resume");
  });

  it('active uses bg-deck-active', () => {
    expect(mk(sess('idle'), { active: true }).get('[data-nav]').classes()).toContain('bg-deck-active');
  });

  it('emits open on click and menu on contextmenu', async () => {
    const w = mk(sess('idle'));
    await w.get('[data-nav]').trigger('click');
    expect(w.emitted('open')).toBeTruthy();
    await w.get('[data-nav]').trigger('contextmenu');
    expect(w.emitted('menu')).toBeTruthy();
  });

  it('rename: Enter saves, Escape cancels', async () => {
    const w = mk(sess('idle'), { renaming: true });
    const input = w.get('input');
    await input.setValue('New name');
    await input.trigger('keydown', { key: 'Enter' });
    expect(w.emitted('rename')![0]).toEqual(['New name']);
    await input.trigger('keydown', { key: 'Escape' });
    expect(w.emitted('cancel-rename')).toBeTruthy();
  });

  it('is draggable and carries the deck id', async () => {
    const w = mk(sess('idle'));
    const row = w.get('[data-nav]');
    expect(row.attributes('draggable')).toBe('true');
    const setData = vi.fn();
    const dataTransfer = { setData, effectAllowed: '' };
    await row.trigger('dragstart', { dataTransfer });
    expect(setData).toHaveBeenCalledWith('application/x-deck-session', '{"deck_id":"d1"}');
    expect(dataTransfer.effectAllowed).toBe('move');
  });
});

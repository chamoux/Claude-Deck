import { describe, expect, it } from 'vitest';
import AsleepPanel from './AsleepPanel.vue';
import { buildDeck, mountWithDeck, sess } from './paneTestKit';

describe('AsleepPanel', () => {
  it('shows the name, the asleep text and a Wake button that wakes the session', async () => {
    const { t, deck } = buildDeck({ sessions: [sess('a', 'p1', 'asleep', 'Alpha one')] });
    const w = mountWithDeck(deck, AsleepPanel, { deckId: 'a' });
    expect(w.text()).toContain('Alpha one');
    expect(w.text()).toContain('This session is asleep');
    expect(w.find('[data-testid="asleep-error"]').exists()).toBe(false);
    await w.get('[data-testid="wake"]').trigger('click');
    expect(t.sent).toEqual([{ type: 'wake', deck_id: 'a' }]);
    w.unmount();
  });

  it('shows the session error in danger colour', () => {
    const { deck } = buildDeck({ sessions: [sess('a', 'p1', 'asleep', null, 'Cannot resume')] });
    const w = mountWithDeck(deck, AsleepPanel, { deckId: 'a' });
    const err = w.get('[data-testid="asleep-error"]');
    expect(err.text()).toBe('Cannot resume');
    expect(err.classes()).toContain('text-deck-danger');
    expect(w.text()).toContain('New session');
    w.unmount();
  });
});

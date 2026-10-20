import { flushPromises, mount } from '@vue/test-utils';
import { defineComponent, h } from 'vue';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { Settings } from '../protocol';
import { fakeTransport } from '../transport';
import { createDeck, provideDeck } from '../useDeck';
import SettingsDialog from './SettingsDialog.vue';

const openMock = vi.fn();
vi.mock('@tauri-apps/plugin-dialog', () => ({ open: (...a: unknown[]) => openMock(...a) }));

const base: Settings = { auto_sleep_minutes: 30, claude_path: null, window: { x: 1, y: 2, width: 3, height: 4 }, shell: 'power_shell' };

function setup(settings: Settings = base) {
  const t = fakeTransport();
  const deck = createDeck(t);
  deck.start();
  t.emit({ type: 'state', state: { projects: [], sessions: [], shells: [], settings, missing_folders: [] } });
  t.sent.length = 0;
  const onClose = vi.fn();
  const Host = defineComponent({
    setup() {
      provideDeck(deck);
      return () => h(SettingsDialog, { open: true, onClose });
    },
  });
  const w = mount(Host, { attachTo: document.body });
  return { t, w, onClose };
}

const save = (w: ReturnType<typeof setup>['w']) => w.findAll('button').find((b) => b.text() === 'Save')!;

beforeEach(() => openMock.mockReset());
afterEach(() => {
  document.body.innerHTML = '';
});

describe('SettingsDialog', () => {
  it('is an accessible modal and loads current settings', async () => {
    const { w } = setup({ ...base, claude_path: 'C:\\x\\claude.exe', shell: 'git_bash' });
    await w.vm.$nextTick();
    const dlg = w.get('[role=dialog]');
    expect(dlg.attributes('aria-modal')).toBe('true');
    expect(w.get(`#${dlg.attributes('aria-labelledby')}`).text()).toBe('Settings');
    expect((w.get('#set-minutes').element as HTMLInputElement).value).toBe('30');
    expect((w.get('#set-path').element as HTMLInputElement).value).toBe('C:\\x\\claude.exe');
    expect((w.get('#set-shell').element as HTMLSelectElement).value).toBe('git_bash');
  });

  it.each([['4'], ['241'], ['']])('minutes %j is invalid: error shown and Save disabled', async (v) => {
    const { w } = setup();
    await w.get('#set-minutes').setValue(v);
    expect(w.get('#set-minutes-err').text()).toContain('5 to 240');
    expect(save(w).attributes('disabled')).toBeDefined();
  });

  it.each([['5'], ['240']])('boundary %s is valid', async (v) => {
    const { w } = setup();
    await w.get('#set-minutes').setValue(v);
    expect(w.get('#set-minutes-err').text()).toBe('');
    expect(save(w).attributes('disabled')).toBeUndefined();
  });

  it('Save sends SetSettings with minutes, path and shell, keeping window, then closes', async () => {
    const { t, w, onClose } = setup();
    await w.get('#set-minutes').setValue('45');
    await w.get('#set-path').setValue('  C:\\bin\\claude.exe ');
    await w.get('#set-shell').setValue('cmd');
    await save(w).trigger('click');
    expect(t.sent).toEqual([
      { type: 'set_settings', settings: { ...base, auto_sleep_minutes: 45, claude_path: 'C:\\bin\\claude.exe', shell: 'cmd' } },
    ]);
    expect(onClose).toHaveBeenCalledOnce();
  });

  it('Off sends null minutes and an empty path sends null; Off ignores an invalid minutes value', async () => {
    const { t, w } = setup();
    await w.get('#set-minutes').setValue('1');
    await w.get('#set-off').trigger('click');
    expect(w.get('#set-off').attributes('aria-checked')).toBe('false');
    expect(save(w).attributes('disabled')).toBeUndefined();
    await save(w).trigger('click');
    expect(t.sent).toEqual([{ type: 'set_settings', settings: { ...base, auto_sleep_minutes: null, claude_path: null } }]);
  });

  it('sends git_bash for Git Bash', async () => {
    const { t, w } = setup();
    await w.get('#set-shell').setValue('git_bash');
    await save(w).trigger('click');
    expect(t.sent[0]).toMatchObject({ settings: { shell: 'git_bash' } });
  });

  it('Browse fills the path from the dialog plugin', async () => {
    openMock.mockResolvedValue('C:\\picked\\claude.exe');
    const { w } = setup();
    await w.findAll('button').find((b) => b.text() === 'Browse')!.trigger('click');
    await flushPromises();
    expect(openMock).toHaveBeenCalledWith({ filters: [{ name: 'claude', extensions: ['exe'] }] });
    expect((w.get('#set-path').element as HTMLInputElement).value).toBe('C:\\picked\\claude.exe');
  });

  it('Browse cancelled leaves the path alone', async () => {
    openMock.mockResolvedValue(null);
    const { w } = setup({ ...base, claude_path: 'keep' });
    await w.findAll('button').find((b) => b.text() === 'Browse')!.trigger('click');
    await flushPromises();
    expect((w.get('#set-path').element as HTMLInputElement).value).toBe('keep');
  });

  it('Cancel and Escape emit close without saving', async () => {
    const { t, w, onClose } = setup();
    await w.findAll('button').find((b) => b.text() === 'Cancel')!.trigger('click');
    await w.get('[role=dialog]').trigger('keydown', { key: 'Escape' });
    expect(onClose).toHaveBeenCalledTimes(2);
    expect(t.sent).toEqual([]);
  });
});

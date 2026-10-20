import type { Deck } from './useDeck';

/** Opens the native folder picker and adds the chosen folder as a project. */
export async function pickAndAddProject(deck: Deck): Promise<void> {
  const { open } = await import('@tauri-apps/plugin-dialog');
  const picked = await open({ directory: true });
  if (typeof picked === 'string' && picked) deck.addProject(picked);
}

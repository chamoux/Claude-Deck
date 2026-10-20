import { ref } from 'vue';
import type { TabRef } from './layout';

export const TAB_MIME = 'application/x-deck-tab';
export const SESSION_MIME = 'application/x-deck-session';

/** The deck drag in flight (a pane tab or a sidebar session), or null. Readable during dragover, unlike dataTransfer. */
export const activeDrag = ref<TabRef | null>(null);

/** Reads a TabRef out of a drag's data, accepting both deck formats. */
export function readDragTab(dt: DataTransfer | null): TabRef | null {
  if (!dt) return null;
  try {
    const tab = dt.getData(TAB_MIME);
    if (tab) {
      const v = JSON.parse(tab) as Partial<TabRef> | null;
      if (v?.kind === 'session' && typeof v.deck_id === 'string') return { kind: 'session', deck_id: v.deck_id };
      if (v?.kind === 'changes' && typeof v.project_id === 'string') return { kind: 'changes', project_id: v.project_id };
    }
    const sess = dt.getData(SESSION_MIME);
    if (sess) {
      const v = JSON.parse(sess) as { deck_id?: unknown } | null;
      if (typeof v?.deck_id === 'string' && v.deck_id !== '') return { kind: 'session', deck_id: v.deck_id };
    }
  } catch {
    // malformed payload: not a deck drag
  }
  return null;
}

/** True when the drag carries one of the deck formats (checked from `types`, which dragover can read). */
export function isDeckDrag(dt: DataTransfer | null): boolean {
  const types = Array.from(dt?.types ?? []);
  return types.includes(TAB_MIME) || types.includes(SESSION_MIME);
}

function onDragStart(e: DragEvent): void {
  // Runs after the source element's own handler (bubble phase), so its data is set and readable here.
  activeDrag.value = readDragTab(e.dataTransfer);
}
const clear = (): void => {
  activeDrag.value = null;
};

let users = 0;

/**
 * Tracks deck drags window-wide so every pane can show its drop overlay while a drag is active.
 * Reference counted; call from onMounted and the returned stop from onBeforeUnmount.
 */
export function trackDeckDrags(): () => void {
  if (users++ === 0) {
    window.addEventListener('dragstart', onDragStart);
    window.addEventListener('dragend', clear);
    window.addEventListener('drop', clear);
  }
  let stopped = false;
  return () => {
    if (stopped) return;
    stopped = true;
    if (--users === 0) {
      window.removeEventListener('dragstart', onDragStart);
      window.removeEventListener('dragend', clear);
      window.removeEventListener('drop', clear);
      activeDrag.value = null;
    }
  };
}

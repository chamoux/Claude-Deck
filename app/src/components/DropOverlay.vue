<script lang="ts">
import type { DropZone } from '../layout';

/** The 25% edge bands are split zones, the rest is the centre; in a corner the nearer edge wins. */
export function zoneAt(x: number, y: number, rect: { left: number; top: number; width: number; height: number }): DropZone {
  const w = rect.width || 1;
  const h = rect.height || 1;
  const nx = (x - rect.left) / w;
  const ny = (y - rect.top) / h;
  const edges: [number, DropZone][] = [
    [nx, 'left'],
    [1 - nx, 'right'],
    [ny, 'top'],
    [1 - ny, 'bottom'],
  ];
  let best = edges[0]!;
  for (const e of edges) if (e[0] < best[0]) best = e;
  return best[0] >= 0.25 ? 'center' : best[1];
}
</script>

<script setup lang="ts">
import { computed, ref } from 'vue';
import { useDeck } from '../useDeck';
import type { DropCheck } from '../useDeck';
import { activeDrag, readDragTab } from '../useDeckDrag';

/**
 * Drop layer for one pane. It is an absolute inset-0 layer above the terminal (xterm's canvas would otherwise
 * swallow dragover) and exists only while a deck drag (pane tab or sidebar session) is in flight.
 * Highlights the area the drop would produce, only when `deck.canDropHere` allows it.
 */
const props = defineProps<{ paneId: string }>();
const deck = useDeck();

const zone = ref<DropZone | null>(null);
const check = ref<DropCheck | null>(null);

const allowed = computed(() => zone.value !== null && check.value?.ok === true);
const showHint = computed(() => check.value !== null && !check.value.ok && check.value.reason === 'other-project');

const AREA: Record<DropZone, string> = {
  left: 'inset-y-0 left-0 w-1/2',
  right: 'inset-y-0 right-0 w-1/2',
  top: 'inset-x-0 top-0 h-1/2',
  bottom: 'inset-x-0 bottom-0 h-1/2',
  center: 'inset-0',
};

function zoneOf(e: DragEvent): DropZone {
  return zoneAt(e.clientX, e.clientY, (e.currentTarget as HTMLElement).getBoundingClientRect());
}

function onDragOver(e: DragEvent): void {
  const tab = activeDrag.value;
  if (!tab) return;
  const z = zoneOf(e);
  const c = deck.canDropHere(props.paneId, z, tab);
  zone.value = z;
  check.value = c;
  if (c.ok) {
    e.preventDefault();
    if (e.dataTransfer) e.dataTransfer.dropEffect = 'move';
  }
}

function reset(): void {
  zone.value = null;
  check.value = null;
}

function onDrop(e: DragEvent): void {
  const tab = readDragTab(e.dataTransfer) ?? activeDrag.value;
  const z = zoneOf(e);
  reset();
  if (!tab) return;
  e.preventDefault();
  deck.dropTab(props.paneId, z, tab);
  activeDrag.value = null;
}
</script>

<template>
  <div
    v-if="activeDrag"
    data-testid="drop-overlay"
    class="absolute inset-0 z-20"
    @dragover="onDragOver"
    @dragleave="reset"
    @drop="onDrop"
  >
    <div
      data-testid="drop-area"
      :data-zone="zone ?? undefined"
      :class="[
        'pointer-events-none absolute rounded-md border-2 border-deck-accent bg-deck-accent/15 transition-opacity duration-[120ms] motion-reduce:transition-none',
        AREA[zone ?? 'center'],
        allowed ? 'opacity-100' : 'opacity-0',
      ]"
      aria-hidden="true"
    />
    <div
      v-if="showHint"
      data-testid="drop-hint"
      class="pointer-events-none absolute top-1/2 left-1/2 -translate-x-1/2 -translate-y-1/2 rounded-md border border-deck-border-strong bg-deck-elevated px-3 py-1.5 text-xs text-deck-muted"
    >
      Add sessions from other projects to a group
    </div>
  </div>
</template>

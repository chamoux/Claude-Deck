<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue';
import { MAX_RATIO, MIN_RATIO } from '../layout';
import type { LayoutNode, Path } from '../layout';
import { useDeck } from '../useDeck';
import { trackDeckDrags } from '../useDeckDrag';
import PaneView from './PaneView.vue';
import Splitter from './Splitter.vue';

/**
 * Renders a layout tree recursively: a split is a flex row or column with a Splitter between its two children,
 * a pane is a PaneView. The container size comes from a ResizeObserver; a Splitter drag (cumulative pixels)
 * becomes a ratio and goes to `deck.setRatio(path, ratio)`. The store clamps and debounces the save.
 */
const props = defineProps<{ node: LayoutNode; path: Path }>();
const deck = useDeck();

const el = ref<HTMLElement | null>(null);
const size = ref({ w: 0, h: 0 });
let observer: ResizeObserver | null = null;
let stopDrags: (() => void) | null = null;
let startRatio: number | null = null;

const isRow = computed(() => props.node.type === 'split' && props.node.dir === 'row');

onMounted(() => {
  // Only the root tracks window-wide deck drags; nested LayoutViews would just add counts.
  if (props.path.length === 0) stopDrags = trackDeckDrags();
});

// The split element exists only while the node is a split (a lone pane can later become one).
watch(
  el,
  (node) => {
    observer?.disconnect();
    observer = null;
    if (!node || typeof ResizeObserver === 'undefined') return;
    observer = new ResizeObserver((entries) => {
      const r = entries[entries.length - 1]?.contentRect;
      if (r) size.value = { w: r.width, h: r.height };
    });
    observer.observe(node);
  },
  { flush: 'post' },
);

onBeforeUnmount(() => {
  observer?.disconnect();
  stopDrags?.();
});

function extent(): number {
  const own = isRow.value ? size.value.w : size.value.h;
  if (own > 0) return own;
  const r = el.value?.getBoundingClientRect();
  return (isRow.value ? r?.width : r?.height) ?? 0;
}

function onDrag(delta: number): void {
  if (props.node.type !== 'split') return;
  const total = extent();
  if (total <= 0) return;
  startRatio ??= props.node.ratio;
  deck.setRatio(props.path, Math.min(MAX_RATIO, Math.max(MIN_RATIO, startRatio + delta / total)));
}

function onEnd(): void {
  startRatio = null;
}

function onReset(): void {
  startRatio = null;
  deck.setRatio(props.path, 0.5);
}
</script>

<template>
  <div
    v-if="node.type === 'split'"
    ref="el"
    data-testid="layout-split"
    :data-dir="node.dir"
    :class="['flex h-full min-h-0 w-full min-w-0 flex-1', isRow ? 'flex-row' : 'flex-col']"
  >
    <div class="flex min-h-0 min-w-0" :style="{ flex: `${node.ratio} 1 0%` }">
      <LayoutView :node="node.a" :path="[...path, 'a']" />
    </div>
    <Splitter
      :orientation="isRow ? 'vertical' : 'horizontal'"
      :label="isRow ? 'Resize panes' : 'Resize panes vertically'"
      @drag="onDrag"
      @end="onEnd"
      @reset="onReset"
    />
    <div class="flex min-h-0 min-w-0" :style="{ flex: `${1 - node.ratio} 1 0%` }">
      <LayoutView :node="node.b" :path="[...path, 'b']" />
    </div>
  </div>
  <PaneView v-else :pane="node" />
</template>

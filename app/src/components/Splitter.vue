<script setup lang="ts">
import { ref } from 'vue';

/**
 * Generic splitter: a 1 px line with an invisible 8 px hit area; the accent line fades in after 150 ms.
 * `drag(deltaPx)` carries the CUMULATIVE movement since the gesture started
 * (pointer down, or one key press); positive = to the right (down when horizontal). `end()` closes the gesture.
 * `reset()` is emitted on double click.
 * `orientation` is the orientation of the line: `vertical` (default) drags sideways, `horizontal` drags up and down
 * (arrow keys Up/Down).
 */
const props = withDefaults(defineProps<{ step?: number; label?: string; orientation?: 'vertical' | 'horizontal' }>(), {
  step: 16,
  label: 'Resize panel',
  orientation: 'vertical',
});
const emit = defineEmits<{ drag: [deltaPx: number]; end: []; reset: [] }>();

const active = ref(false);
let start = 0;

const coord = (e: PointerEvent): number => (props.orientation === 'horizontal' ? e.clientY : e.clientX);

function onPointerDown(e: PointerEvent): void {
  if (e.button !== undefined && e.button !== 0) return;
  active.value = true;
  start = coord(e);
  (e.currentTarget as HTMLElement | null)?.setPointerCapture?.(e.pointerId);
  e.preventDefault();
}

function onPointerMove(e: PointerEvent): void {
  if (!active.value) return;
  emit('drag', coord(e) - start);
}

function finish(e: PointerEvent): void {
  if (!active.value) return;
  active.value = false;
  (e.currentTarget as HTMLElement | null)?.releasePointerCapture?.(e.pointerId);
  emit('end');
}

function onKeydown(e: KeyboardEvent): void {
  const [less, more] = props.orientation === 'horizontal' ? ['ArrowUp', 'ArrowDown'] : ['ArrowLeft', 'ArrowRight'];
  if (e.key !== less && e.key !== more) return;
  e.preventDefault();
  emit('drag', e.key === less ? -props.step : props.step);
  emit('end');
}
</script>

<template>
  <div
    role="separator"
    :aria-orientation="orientation"
    :aria-label="label"
    tabindex="0"
    :class="[
      'group relative z-10 shrink-0 touch-none bg-deck-border outline-none',
      orientation === 'horizontal' ? 'h-px w-full cursor-row-resize' : 'w-px cursor-col-resize',
    ]"
    :data-active="active || undefined"
    @pointerdown="onPointerDown"
    @pointermove="onPointerMove"
    @pointerup="finish"
    @pointercancel="finish"
    @keydown="onKeydown"
    @dblclick="emit('reset')"
  >
    <span
      :class="[
        'absolute',
        orientation === 'horizontal' ? 'inset-x-0 -top-[4px] h-2 cursor-row-resize' : 'inset-y-0 -left-[4px] w-2 cursor-col-resize',
      ]"
      aria-hidden="true"
    ></span>
    <span
      :class="[
        'pointer-events-none absolute bg-deck-accent transition-opacity duration-150 motion-reduce:transition-none',
        orientation === 'horizontal' ? 'inset-x-0 -top-px h-0.5' : 'inset-y-0 -left-px w-0.5',
        'opacity-0 group-hover:opacity-100 group-hover:delay-150 group-focus-visible:opacity-100',
        active ? 'opacity-100' : '',
      ]"
      aria-hidden="true"
    ></span>
  </div>
</template>

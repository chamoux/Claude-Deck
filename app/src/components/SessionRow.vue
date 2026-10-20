<script setup lang="ts">
import { computed, nextTick, ref, watch } from 'vue';
import type { SessionView } from '../protocol';
import { dotClass } from './sessionDot';

const props = defineProps<{ session: SessionView; active: boolean; renaming: boolean }>();
const emit = defineEmits<{
  open: [];
  menu: [event: MouseEvent];
  rename: [name: string];
  'cancel-rename': [];
}>();

const input = ref<HTMLInputElement | null>(null);
const draft = ref('');

const label = computed(() => props.session.name || 'New session');
const needs = computed(() => props.session.state === 'needs_you');
const asleep = computed(() => props.session.state === 'asleep');

const dot = computed(() => dotClass(props.session.state));
// Needs you wins over active: the two must never look alike.
const rowClass = computed(() => {
  if (needs.value) return 'bg-needs-soft text-deck-text';
  if (props.active) return 'bg-deck-active font-semibold text-deck-text';
  return 'text-deck-text hover:bg-deck-hover';
});
const barClass = computed(() => (needs.value ? 'bg-dot-needs' : props.active ? 'bg-deck-accent' : ''));

watch(
  () => props.renaming,
  async (on) => {
    if (!on) return;
    draft.value = props.session.name ?? '';
    await nextTick();
    input.value?.focus();
    input.value?.select();
  },
  { immediate: true },
);

function onDragStart(e: DragEvent): void {
  if (!e.dataTransfer) return;
  e.dataTransfer.setData('application/x-deck-session', JSON.stringify({ deck_id: props.session.deck_id }));
  e.dataTransfer.effectAllowed = 'move';
}

function save(): void {
  const name = draft.value.trim();
  if (name) emit('rename', name);
  else emit('cancel-rename');
}
</script>

<template>
  <div>
    <input
      v-if="renaming"
      ref="input"
      v-model="draft"
      type="text"
      aria-label="Session name"
      class="ml-4 h-7 w-[calc(100%-1rem)] rounded-md border border-deck-accent bg-deck-bg px-2 font-sans text-[13px] text-deck-text focus:outline-none focus-visible:ring-2 focus-visible:ring-deck-accent"
      @keydown.enter.prevent="save"
      @keydown.esc.prevent="emit('cancel-rename')"
      @blur="emit('cancel-rename')"
    />
    <button
      v-else
      type="button"
      data-nav
      draggable="true"
      :class="[
        'relative flex min-h-[30px] w-full cursor-pointer flex-wrap items-center gap-x-2 gap-y-0.5 rounded-md py-1 pr-2 pl-6 text-left font-sans text-[13px] transition-colors duration-150 focus:outline-none focus-visible:ring-2 focus-visible:ring-deck-accent',
        rowClass,
      ]"
      @click="emit('open')"
      @dragstart="onDragStart"
      @contextmenu.prevent="emit('menu', $event)"
    >
      <span
        v-if="barClass"
        data-testid="bar"
        :class="['absolute inset-y-1 left-0 w-0.5 rounded-full', barClass]"
        aria-hidden="true"
      />
      <span data-testid="dot" :class="['size-2.5 shrink-0 rounded-full', dot]" aria-hidden="true" />
      <span
        data-testid="name"
        :title="label"
        :class="['min-w-0 flex-[1_1_6rem] truncate', asleep ? 'text-deck-muted' : '']"
        >{{ label }}</span
      >
      <span
        v-if="session.error"
        class="max-w-full shrink-0 truncate rounded bg-deck-danger/15 px-1.5 text-xs leading-4 text-deck-danger"
        :title="session.error"
        >{{ session.error.startsWith('Claude quit') ? 'Not trusted' : "Can't resume" }}</span
      >
      <span
        v-if="needs"
        data-testid="needs-pill"
        class="ml-auto shrink-0 rounded-full bg-dot-needs/20 px-1.5 text-[11px] leading-4 font-semibold text-dot-needs"
        >Needs you</span
      >
    </button>
  </div>
</template>

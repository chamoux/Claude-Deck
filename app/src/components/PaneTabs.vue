<script setup lang="ts">
import { computed, ref } from 'vue';
import { Columns2, FileDiff, Plus, Rows2, X } from 'lucide-vue-next';
import { tabKey } from '../layout';
import type { Pane, TabRef } from '../layout';
import { useDeck } from '../useDeck';
import { TAB_MIME } from '../useDeckDrag';
import ContextMenu from './ContextMenu.vue';
import type { MenuItem } from './ContextMenu.vue';
import { dotClass } from './sessionDot';

/**
 * The 36 px tab strip of one pane. ARIA tablist with manual activation (arrows / Home / End move focus,
 * Enter or Space opens). Tabs are draggable (`application/x-deck-tab` = the JSON TabRef) and have a context menu.
 * The focused pane gets a 2 px accent line across the top of its strip. In a group view each tab also shows
 * its project's name. Closing a tab only removes it from the layout; the session keeps running.
 */
const props = defineProps<{ pane: Pane; focused: boolean }>();
const deck = useDeck();
const root = ref<HTMLElement | null>(null);
const menu = ref<{ x: number; y: number; index: number } | null>(null);

const inGroup = computed(() => deck.selected.value?.kind === 'group');
const projectId = computed(() => (deck.selected.value?.kind === 'project' ? deck.selected.value.id : null));

interface TabVm {
  ref: TabRef;
  key: string;
  index: number;
  active: boolean;
  label: string;
  project: string | null;
  needs: boolean;
  dot: string | null;
  error: string | null;
}

function projectName(id: string | undefined): string | null {
  if (!inGroup.value || id === undefined) return null;
  return deck.projects.value.find((p) => p.id === id)?.name ?? null;
}

const tabs = computed<TabVm[]>(() =>
  props.pane.tabs.map((t, index) => {
    const active = index === props.pane.active;
    if (t.kind === 'changes') {
      return { ref: t, key: tabKey(t), index, active, label: 'Changes', project: projectName(t.project_id), needs: false, dot: null, error: null };
    }
    const s = deck.session(t.deck_id);
    return {
      ref: t,
      key: tabKey(t),
      index,
      active,
      label: s?.name || 'New session',
      project: projectName(s?.project_id),
      needs: s?.state === 'needs_you',
      dot: dotClass(s?.state ?? 'asleep'),
      error: s?.error ?? null,
    };
  }),
);

function changesTotal(t: TabRef): number | null {
  if (t.kind !== 'changes') return null;
  const r = deck.changes[t.project_id];
  return r?.status === 'ok' ? r.list.total : null;
}

function open(t: TabVm): void {
  deck.focusPane(props.pane.id);
  if (t.ref.kind === 'session') deck.activateTab(t.ref.deck_id);
  else deck.setActiveTab(props.pane.id, t.index);
}

function close(t: TabVm): void {
  deck.closeTab(t.key);
}

function onDragStart(e: DragEvent, t: TabVm): void {
  const dt = e.dataTransfer;
  if (!dt) return;
  dt.setData(TAB_MIME, JSON.stringify(t.ref));
  dt.setData('text/plain', t.label);
  dt.effectAllowed = 'move';
}

function onKeydown(e: KeyboardEvent): void {
  const keys = ['ArrowLeft', 'ArrowRight', 'Home', 'End'];
  if (!keys.includes(e.key)) return;
  const els = Array.from(root.value?.querySelectorAll<HTMLElement>('[role="tab"]') ?? []);
  if (els.length === 0) return;
  e.preventDefault();
  const i = els.indexOf(document.activeElement as HTMLElement);
  let next = i < 0 ? 0 : i;
  if (e.key === 'ArrowRight') next = (i + 1) % els.length;
  else if (e.key === 'ArrowLeft') next = (i - 1 + els.length) % els.length;
  else if (e.key === 'Home') next = 0;
  else next = els.length - 1;
  els[next]?.focus();
}

const menuItems = computed<MenuItem[]>(() => {
  const canSplit = props.pane.tabs.length >= 2;
  return [
    { key: 'split-right', label: 'Split right', icon: Columns2, disabled: !canSplit },
    { key: 'split-down', label: 'Split down', icon: Rows2, disabled: !canSplit },
    { key: 'close', label: 'Close tab', icon: X, separator: true },
  ];
});

function onContext(e: MouseEvent, t: TabVm): void {
  e.preventDefault();
  menu.value = { x: e.clientX, y: e.clientY, index: t.index };
}

function onMenuSelect(key: string): void {
  const m = menu.value;
  const t = m ? tabs.value[m.index] : undefined;
  if (!t) return;
  if (key === 'close') {
    close(t);
    return;
  }
  // Splitting acts on the focused pane's active tab, so make the clicked tab that one first.
  deck.focusPane(props.pane.id);
  deck.setActiveTab(props.pane.id, t.index);
  deck.splitFocused(key === 'split-right' ? 'row' : 'column');
}

function newSession(): void {
  if (!projectId.value) return;
  deck.focusPane(props.pane.id);
  deck.newSession(projectId.value);
}

const tabClass = (t: TabVm): string[] => [
  'group relative flex max-w-[240px] min-w-0 shrink-0 items-center border-r border-deck-border transition-colors duration-150',
  t.needs
    ? 'bg-needs-soft text-dot-needs'
    : t.active
      ? 'bg-deck-term font-medium text-deck-text'
      : 'text-deck-muted hover:bg-deck-hover hover:text-deck-text',
];
</script>

<template>
  <div class="relative flex h-9 shrink-0 items-stretch border-b border-deck-border bg-deck-panel" data-testid="pane-tabs">
    <span
      v-if="focused"
      data-testid="focus-line"
      class="pointer-events-none absolute inset-x-0 top-0 z-10 h-0.5 bg-deck-accent"
      aria-hidden="true"
    />
    <div
      ref="root"
      role="tablist"
      aria-label="Tabs"
      class="flex min-w-0 flex-1 items-stretch overflow-x-auto"
      @keydown="onKeydown"
    >
      <div
        v-for="t in tabs"
        :key="t.key"
        :data-testid="'tab-' + (t.ref.kind === 'session' ? t.ref.deck_id : 'changes')"
        :data-active="t.active || undefined"
        draggable="true"
        :class="tabClass(t)"
        @dragstart="onDragStart($event, t)"
        @contextmenu="onContext($event, t)"
        @auxclick.middle.prevent="close(t)"
        @mousedown.middle.prevent
      >
        <span
          v-if="t.needs"
          class="pointer-events-none absolute inset-x-0 top-0 z-20 h-0.5 bg-dot-needs"
          aria-hidden="true"
        />
        <button
          type="button"
          role="tab"
          :aria-selected="t.active"
          :tabindex="t.active ? 0 : -1"
          class="flex min-w-0 flex-1 cursor-pointer items-center gap-2 self-stretch pr-1 pl-3 text-[13px] focus:outline-none focus-visible:ring-2 focus-visible:ring-deck-accent focus-visible:ring-inset"
          @click="open(t)"
        >
          <FileDiff v-if="t.ref.kind === 'changes'" :size="14" :stroke-width="1.75" class="shrink-0" aria-hidden="true" />
          <span v-else data-testid="tab-dot" :class="['size-2 shrink-0 rounded-full', t.dot]" aria-hidden="true" />
          <span class="min-w-0 truncate" :title="t.error ?? t.label">{{ t.label }}</span>
          <span
            v-if="changesTotal(t.ref) !== null"
            data-testid="changes-count"
            class="rounded-full bg-deck-border px-1.5 text-[11px] leading-4 font-medium text-deck-muted"
          >{{ changesTotal(t.ref) }}</span>
          <span v-if="t.project" data-testid="tab-project" class="shrink-0 text-[11px] font-normal text-deck-faint">{{ t.project }}</span>
          <span v-if="t.needs" class="shrink-0 text-xs font-semibold">Needs you</span>
          <span v-if="t.error" class="shrink-0 text-xs text-deck-danger">!</span>
        </button>
        <button
          type="button"
          tabindex="-1"
          :aria-label="'Close tab ' + t.label"
          :title="'Close tab ' + t.label"
          :class="[
            'mr-1.5 flex size-5 shrink-0 cursor-pointer items-center justify-center rounded text-deck-muted transition-colors duration-150 hover:bg-deck-border hover:text-deck-text focus:outline-none focus-visible:opacity-100 focus-visible:ring-2 focus-visible:ring-deck-accent',
            t.active ? 'opacity-100' : 'opacity-0 group-focus-within:opacity-100 group-hover:opacity-100',
          ]"
          @click="close(t)"
        >
          <X :size="14" :stroke-width="1.75" aria-hidden="true" />
        </button>
      </div>
    </div>
    <button
      v-if="projectId"
      type="button"
      data-testid="new-session"
      aria-label="New Claude session"
      title="New Claude session (Ctrl+T)"
      class="flex w-9 shrink-0 cursor-pointer items-center justify-center text-deck-muted transition-colors duration-150 hover:bg-deck-hover hover:text-deck-text focus:outline-none focus-visible:ring-2 focus-visible:ring-deck-accent focus-visible:ring-inset"
      @click="newSession"
    >
      <Plus :size="16" :stroke-width="1.75" aria-hidden="true" />
    </button>
    <ContextMenu
      v-if="menu"
      :x="menu.x"
      :y="menu.y"
      :items="menuItems"
      @select="onMenuSelect"
      @close="menu = null"
    />
  </div>
</template>

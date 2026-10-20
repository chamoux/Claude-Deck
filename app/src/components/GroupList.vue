<script setup lang="ts">
import { LayoutGrid, Pencil, Plus, Trash2 } from 'lucide-vue-next';
import { nextTick, ref } from 'vue';
import { allTabs } from '../layout';
import type { Group } from '../protocol';
import { useDeck } from '../useDeck';
import ContextMenu from './ContextMenu.vue';
import type { MenuItem } from './ContextMenu.vue';

const SESSION_MIME = 'application/x-deck-session';
const TAB_MIME = 'application/x-deck-tab';

const deck = useDeck();
const renamingId = ref<string | null>(null);
const draft = ref('');
const dragOverId = ref<string | null>(null);
const menu = ref<{ x: number; y: number; id: string } | null>(null);
const input = ref<HTMLInputElement | HTMLInputElement[] | null>(null);

const menuItems: MenuItem[] = [
  { key: 'tile', label: 'Arrange side by side', icon: LayoutGrid },
  { key: 'rename', label: 'Rename', icon: Pencil },
  { key: 'delete', label: 'Delete group', icon: Trash2, danger: true, separator: true },
];

/** Session ids in a group's layout (changes tabs are not sessions). */
function sessionIds(g: Group): string[] {
  const ids: string[] = [];
  for (const t of allTabs(deck.layoutOf({ kind: 'group', id: g.id }))) {
    if (t.kind === 'session') ids.push(t.deck_id);
  }
  return ids;
}
function count(g: Group): number {
  return sessionIds(g).length;
}
function needs(g: Group): number {
  return sessionIds(g).filter((id) => deck.session(id)?.state === 'needs_you').length;
}
function isActive(g: Group): boolean {
  const s = deck.selected.value;
  return s?.kind === 'group' && s.id === g.id;
}

function rowClass(g: Group): string {
  if (dragOverId.value === g.id) return 'bg-deck-active text-deck-text outline-2 -outline-offset-2 outline-dashed outline-deck-accent';
  if (isActive(g)) return 'bg-deck-active font-semibold text-deck-text';
  return 'text-deck-muted hover:bg-deck-hover';
}

async function startRename(g: Group): Promise<void> {
  draft.value = g.name;
  renamingId.value = g.id;
  await nextTick();
  const el = Array.isArray(input.value) ? input.value[0] : input.value;
  el?.focus();
  el?.select();
}

function save(g: Group): void {
  if (renamingId.value !== g.id) return;
  const name = draft.value.trim();
  renamingId.value = null;
  if (name && name !== g.name) deck.renameGroup(g.id, name);
}

function onSelect(key: string): void {
  const g = deck.groups.value.find((x) => x.id === menu.value?.id);
  if (!g) return;
  if (key === 'tile') deck.tileGroup(g.id);
  else if (key === 'rename') void startRename(g);
  else if (key === 'delete') deck.removeGroup(g.id);
}

function accepts(e: DragEvent): boolean {
  const types = Array.from(e.dataTransfer?.types ?? []);
  return types.includes(SESSION_MIME) || types.includes(TAB_MIME);
}
function onDragOver(e: DragEvent, g: Group): void {
  if (!accepts(e)) return;
  e.preventDefault();
  if (e.dataTransfer) e.dataTransfer.dropEffect = 'move';
  dragOverId.value = g.id;
}
function onDragLeave(g: Group): void {
  if (dragOverId.value === g.id) dragOverId.value = null;
}
function onDrop(e: DragEvent, g: Group): void {
  dragOverId.value = null;
  if (!accepts(e)) return;
  e.preventDefault();
  try {
    const dt = e.dataTransfer;
    const raw = dt?.getData(SESSION_MIME);
    if (raw) {
      const { deck_id } = JSON.parse(raw) as { deck_id?: string };
      if (deck_id) deck.addToGroup(g.id, deck_id);
      return;
    }
    const tab = JSON.parse(dt?.getData(TAB_MIME) || 'null') as { kind?: string; deck_id?: string } | null;
    if (tab?.kind === 'session' && tab.deck_id) deck.addToGroup(g.id, tab.deck_id);
  } catch {
    // Malformed drag payload: ignore.
  }
}
</script>

<template>
  <section aria-label="Groups" data-testid="groups">
    <header class="flex items-center justify-between py-2 pr-2 pl-3">
      <h2 class="text-[11px] font-semibold tracking-wider text-deck-faint uppercase">GROUPS</h2>
      <button
        type="button"
        data-testid="new-group"
        aria-label="New group"
        title="New group"
        class="flex size-7 cursor-pointer items-center justify-center rounded-md text-deck-muted transition-colors duration-150 hover:bg-deck-hover hover:text-deck-text focus:outline-none focus-visible:ring-2 focus-visible:ring-deck-accent"
        @click="deck.newGroup()"
      >
        <Plus :size="16" :stroke-width="1.75" aria-hidden="true" />
      </button>
    </header>

    <p v-if="deck.groups.value.length === 0" class="px-3 pb-3 text-xs text-deck-faint">Combine sessions from any projects</p>

    <ul v-else class="flex flex-col gap-0.5 px-1.5 pb-2">
      <li v-for="g in deck.groups.value" :key="g.id" :data-testid="'group-' + g.id">
        <input
          v-if="renamingId === g.id"
          ref="input"
          v-model="draft"
          type="text"
          aria-label="Group name"
          class="h-7 w-full rounded-md border border-deck-accent bg-deck-bg px-2 font-sans text-[13px] text-deck-text focus:outline-none focus-visible:ring-2 focus-visible:ring-deck-accent"
          @keydown.enter.prevent="save(g)"
          @keydown.esc.prevent="renamingId = null"
          @blur="renamingId = null"
        />
        <button
          v-else
          type="button"
          data-nav
          :class="[
            'relative flex h-[30px] w-full cursor-pointer items-center gap-2 rounded-md px-2 text-left transition-colors duration-150 focus:outline-none focus-visible:ring-2 focus-visible:ring-deck-accent',
            rowClass(g),
          ]"
          @click="deck.selectGroup(g.id)"
          @dblclick="startRename(g)"
          @contextmenu.prevent="menu = { x: $event.clientX, y: $event.clientY, id: g.id }"
          @dragenter="onDragOver($event, g)"
          @dragover="onDragOver($event, g)"
          @dragleave="onDragLeave(g)"
          @drop="onDrop($event, g)"
        >
          <span
            v-if="isActive(g)"
            data-testid="bar"
            class="absolute inset-y-1 left-0 w-0.5 rounded-full bg-deck-accent"
            aria-hidden="true"
          />
          <LayoutGrid :size="14" :stroke-width="1.75" class="shrink-0 text-deck-faint" aria-hidden="true" />
          <span class="min-w-0 flex-1 truncate font-medium" :title="g.name">{{ g.name }}</span>
          <span
            v-if="needs(g) > 0"
            data-testid="group-needs"
            :title="`${needs(g)} need you`"
            class="shrink-0 rounded-full bg-dot-needs/20 px-1.5 text-[11px] leading-4 font-semibold text-dot-needs"
            >{{ needs(g) }}</span
          >
          <span data-testid="group-count" class="min-w-4 shrink-0 text-right font-mono text-xs font-normal text-deck-faint">{{
            count(g)
          }}</span>
        </button>
      </li>
    </ul>

    <ContextMenu
      v-if="menu"
      :x="menu.x"
      :y="menu.y"
      :items="menuItems"
      @select="onSelect"
      @close="menu = null"
    />
  </section>
</template>

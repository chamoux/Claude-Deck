<script setup lang="ts">
import { ChevronRight, Folder, FolderOpen, FolderX, History, LayoutGrid, Moon, Pencil, Plus, Trash2, X } from 'lucide-vue-next';
import { computed, reactive, ref } from 'vue';
import { panes } from '../layout';
import { pickAndAddProject } from '../pickFolder';
import { useDeck } from '../useDeck';
import ContextMenu from './ContextMenu.vue';
import type { MenuItem } from './ContextMenu.vue';
import GroupList from './GroupList.vue';
import SessionRow from './SessionRow.vue';

const emit = defineEmits<{ 'resume-past': [projectId: string] }>();

const deck = useDeck();
const root = ref<HTMLElement | null>(null);
const collapsed = reactive<Record<string, boolean>>({});
const renamingId = ref<string | null>(null);

type MenuTarget = { x: number; y: number; kind: 'project' | 'session'; id: string };
const menu = ref<MenuTarget | null>(null);

const missing = computed(() => new Set(deck.state.value?.missing_folders ?? []));

function openMenu(e: MouseEvent, kind: MenuTarget['kind'], id: string): void {
  menu.value = { x: e.clientX, y: e.clientY, kind, id };
}

const menuItems = computed<MenuItem[]>(() => {
  const m = menu.value;
  if (!m) return [];
  if (m.kind === 'project') {
    return [
      { key: 'new', label: 'New Claude session', icon: Plus },
      { key: 'resume', label: 'Resume past session…', icon: History },
      { key: 'explorer', label: 'Open in File Explorer', icon: FolderOpen, separator: true },
      { key: 'remove', label: 'Remove from list', icon: Trash2, danger: true, separator: true },
    ];
  }
  const asleep = deck.session(m.id)?.state === 'asleep';
  const groupItems: MenuItem[] = deck.groups.value.map((g) => ({ key: 'group:' + g.id, label: g.name, icon: LayoutGrid }));
  groupItems.push({ key: 'new-group', label: 'New group…', icon: Plus, separator: groupItems.length > 0 });
  return [
    { key: 'rename', label: 'Rename', icon: Pencil },
    { key: 'add-to-group', label: 'Add to group', icon: LayoutGrid, children: groupItems },
    { key: 'sleep', label: 'Sleep now', icon: Moon, disabled: asleep },
    { key: 'close', label: 'Close session', icon: X, separator: true },
  ];
});

async function onSelect(key: string): Promise<void> {
  const m = menu.value;
  if (!m) return;
  if (key.startsWith('group:')) {
    deck.addToGroup(key.slice('group:'.length), m.id);
    return;
  }
  switch (key) {
    case 'new-group':
      deck.newGroup(undefined, { addSession: m.id });
      break;
    case 'new':
      deck.newSession(m.id);
      break;
    case 'resume':
      emit('resume-past', m.id);
      break;
    case 'explorer': {
      const path = deck.projects.value.find((p) => p.id === m.id)?.path;
      if (path) {
        const { openPath } = await import('@tauri-apps/plugin-opener');
        await openPath(path);
      }
      break;
    }
    case 'remove':
      deck.removeProject(m.id);
      break;
    case 'rename':
      renamingId.value = m.id;
      break;
    case 'sleep':
      deck.sleep(m.id);
      break;
    case 'close':
      deck.closeSession(m.id);
      break;
  }
}

function toggleProject(id: string): void {
  deck.selectProject(id);
  collapsed[id] = !collapsed[id];
}

function addProject(): void {
  void pickAndAddProject(deck);
}

function saveRename(deckId: string, name: string): void {
  deck.rename(deckId, name);
  renamingId.value = null;
}

/** The focused pane's active tab in the current view. */
const activeSessionId = computed<string | null>(() => {
  const layout = deck.currentLayout.value;
  const list = panes(layout);
  const pane = list.find((p) => p.id === deck.focusedPaneId.value) ?? list[0];
  const t = pane?.tabs[pane.active];
  return t?.kind === 'session' ? t.deck_id : null;
});

function isProjectSelected(id: string): boolean {
  const s = deck.selected.value;
  return s?.kind === 'project' && s.id === id;
}

function onKeydown(e: KeyboardEvent): void {
  if (e.key !== 'ArrowDown' && e.key !== 'ArrowUp') return;
  const rows = Array.from(root.value?.querySelectorAll<HTMLElement>('[data-nav]') ?? []);
  if (rows.length === 0) return;
  e.preventDefault();
  const i = rows.indexOf(document.activeElement as HTMLElement);
  const delta = e.key === 'ArrowDown' ? 1 : -1;
  const next = i === -1 ? (delta > 0 ? 0 : rows.length - 1) : Math.min(rows.length - 1, Math.max(0, i + delta));
  rows[next].focus();
}
</script>

<template>
  <nav
    ref="root"
    aria-label="Projects"
    class="flex h-full w-full flex-col overflow-y-auto border-r border-deck-border bg-deck-sidebar font-sans text-[13px] text-deck-text select-none"
    @keydown="onKeydown"
  >
    <GroupList />

    <header class="flex items-center justify-between py-2 pr-2 pl-3">
      <h2 class="text-[11px] font-semibold tracking-wider text-deck-faint uppercase">PROJECTS</h2>
      <button
        type="button"
        data-testid="add-project"
        aria-label="Add project folder"
        title="Add project folder"
        class="flex size-7 cursor-pointer items-center justify-center rounded-md text-deck-muted transition-colors duration-150 hover:bg-deck-hover hover:text-deck-text focus:outline-none focus-visible:ring-2 focus-visible:ring-deck-accent"
        @click="addProject"
      >
        <Plus :size="16" :stroke-width="1.75" aria-hidden="true" />
      </button>
    </header>

    <ul class="flex flex-col gap-0.5 px-1.5 pb-3">
      <li v-for="p in deck.projects.value" :key="p.id" :data-testid="'project-' + p.id">
        <button
          type="button"
          data-nav
          :aria-expanded="!collapsed[p.id]"
          :title="p.path"
          :class="[
            'flex h-[30px] w-full cursor-pointer items-center gap-2 rounded-md px-2 text-left font-medium transition-colors duration-150 hover:bg-deck-hover focus:outline-none focus-visible:ring-2 focus-visible:ring-deck-accent',
            isProjectSelected(p.id) ? 'text-deck-text' : 'text-deck-muted',
            missing.has(p.id) ? 'opacity-50' : '',
          ]"
          @click="toggleProject(p.id)"
          @contextmenu.prevent="openMenu($event, 'project', p.id)"
        >
          <ChevronRight
            :size="14"
            :stroke-width="1.75"
            class="shrink-0 text-deck-faint transition-transform duration-150"
            :class="collapsed[p.id] ? '' : 'rotate-90'"
            aria-hidden="true"
          />
          <FolderX v-if="missing.has(p.id)" :size="14" :stroke-width="1.75" class="shrink-0 text-deck-danger" aria-hidden="true" />
          <FolderOpen v-else-if="!collapsed[p.id]" :size="14" :stroke-width="1.75" class="shrink-0 text-deck-faint" aria-hidden="true" />
          <Folder v-else :size="14" :stroke-width="1.75" class="shrink-0 text-deck-faint" aria-hidden="true" />
          <span class="min-w-[4.5rem] flex-1 truncate" :title="p.path">{{ p.name }}</span>
          <span v-if="missing.has(p.id)" class="shrink-0 text-[11px] font-normal text-deck-danger">Folder missing</span>
          <span
            v-if="(deck.needsYouByProject.value[p.id] ?? 0) > 0"
            data-testid="needs-badge"
            class="shrink-0 rounded-full bg-dot-needs/20 px-1.5 text-[11px] leading-4 font-semibold text-dot-needs"
            >{{ deck.needsYouByProject.value[p.id] }}</span
          >
          <span data-testid="count" class="min-w-4 shrink-0 text-right font-mono text-xs font-normal text-deck-faint">{{
            deck.sessionsOf(p.id).length
          }}</span>
        </button>

        <ul v-if="!collapsed[p.id]" class="mt-0.5 flex flex-col gap-0.5">
          <li v-for="s in deck.sessionsOf(p.id)" :key="s.deck_id" :data-testid="'session-' + s.deck_id">
            <SessionRow
              :session="s"
              :active="activeSessionId === s.deck_id"
              :renaming="renamingId === s.deck_id"
              @open="deck.openSession(s.deck_id)"
              @menu="openMenu($event, 'session', s.deck_id)"
              @rename="saveRename(s.deck_id, $event)"
              @cancel-rename="renamingId = null"
            />
          </li>
        </ul>
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
  </nav>
</template>

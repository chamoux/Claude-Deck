<script setup lang="ts">
import { ChevronRight } from 'lucide-vue-next';
import type { Component } from 'vue';
import { nextTick, onBeforeUnmount, onMounted, ref } from 'vue';

export interface MenuItem {
  key: string;
  label: string;
  disabled?: boolean;
  icon?: Component;
  danger?: boolean;
  /** Draw a 1 px separator above this item (start of a new group). */
  separator?: boolean;
  /** Makes this item a submenu parent: it opens a nested menu to the right and never selects itself. */
  children?: MenuItem[];
}

/** Hover delay before a submenu opens, and the gap kept to the window edge. */
const HOVER_MS = 150;
const EDGE = 4;

const props = defineProps<{ x: number; y: number; items: MenuItem[] }>();
const emit = defineEmits<{ select: [key: string]; close: [] }>();

const menu = ref<HTMLElement | null>(null);

/** Items of the menu level that `scope` belongs to (the root menu or the open submenu). */
function buttons(scope: ParentNode | null = menu.value): HTMLElement[] {
  return Array.from(scope?.querySelectorAll<HTMLElement>(':scope > [role="menuitem"]:not([disabled])') ?? []);
}

function move(delta: number, from: HTMLElement): void {
  const list = buttons(from.closest('[role="menu"]'));
  if (list.length === 0) return;
  const i = list.indexOf(from);
  const next = i === -1 ? (delta > 0 ? 0 : list.length - 1) : (i + delta + list.length) % list.length;
  list[next].focus();
}

const openKey = ref<string | null>(null);
const subStyle = ref<Record<string, string>>({});
const sub = ref<HTMLElement | null>(null);
let hoverTimer: ReturnType<typeof setTimeout> | undefined;

function itemEl(key: string): HTMLElement | null {
  return menu.value?.querySelector<HTMLElement>(`:scope > [data-key="${key}"]`) ?? null;
}

async function openSub(item: MenuItem, focusFirst: boolean): Promise<void> {
  clearTimeout(hoverTimer);
  const trigger = itemEl(item.key);
  if (!trigger || item.disabled) return;
  const r = trigger.getBoundingClientRect();
  openKey.value = item.key;
  subStyle.value = { left: `${r.right}px`, top: `${r.top - 5}px` };
  await nextTick();
  const el = sub.value;
  if (el) {
    // Keep the nested menu inside the window: flip to the left of the parent, and shift up if it would overflow.
    const s = el.getBoundingClientRect();
    let left = r.right;
    if (left + s.width > window.innerWidth - EDGE) left = Math.max(EDGE, r.left - s.width);
    let top = r.top - 5;
    if (top + s.height > window.innerHeight - EDGE) top = Math.max(EDGE, window.innerHeight - s.height - EDGE);
    subStyle.value = { left: `${left}px`, top: `${top}px` };
  }
  if (focusFirst) buttons(sub.value)[0]?.focus();
}

function closeSub(refocus: boolean): void {
  clearTimeout(hoverTimer);
  const key = openKey.value;
  openKey.value = null;
  if (refocus && key) itemEl(key)?.focus();
}

function onHover(item: MenuItem): void {
  clearTimeout(hoverTimer);
  if (item.children && !item.disabled) {
    if (openKey.value !== item.key) hoverTimer = setTimeout(() => void openSub(item, false), HOVER_MS);
  } else if (openKey.value) {
    closeSub(false);
  }
}

function findItem(el: HTMLElement | null): MenuItem | undefined {
  const key = el?.getAttribute('data-key');
  return props.items.find((i) => i.key === key);
}

function onKeydown(e: KeyboardEvent): void {
  // Tests and some browsers dispatch on the menu itself: fall back to the focused item.
  const raw = e.target as HTMLElement;
  const target = raw.getAttribute('role') === 'menuitem' ? raw : ((document.activeElement as HTMLElement | null) ?? raw);
  const inSub = target.closest('[data-submenu]') !== null;
  if (e.key === 'Escape' || (e.key === 'ArrowLeft' && inSub)) {
    e.preventDefault();
    if (inSub) closeSub(true);
    else emit('close');
  } else if (e.key === 'ArrowRight') {
    const item = inSub ? undefined : findItem(target);
    if (item?.children) {
      e.preventDefault();
      void openSub(item, true);
    }
  } else if (e.key === 'ArrowDown') {
    e.preventDefault();
    move(1, target);
  } else if (e.key === 'ArrowUp') {
    e.preventDefault();
    move(-1, target);
  } else if (e.key === 'Tab') {
    e.preventDefault();
    emit('close');
  }
}

function pick(item: MenuItem): void {
  if (item.disabled) return;
  if (item.children) {
    void openSub(item, true);
    return;
  }
  emit('select', item.key);
  emit('close');
}

function onOutside(e: Event): void {
  if (menu.value && !menu.value.contains(e.target as Node)) emit('close');
}

function onFocusOut(e: FocusEvent): void {
  const next = e.relatedTarget as Node | null;
  if (next && !menu.value?.contains(next)) emit('close');
}

onMounted(() => {
  document.addEventListener('pointerdown', onOutside, true);
  window.addEventListener('blur', onWindowBlur);
  buttons()[0]?.focus();
});
onBeforeUnmount(() => {
  clearTimeout(hoverTimer);
  document.removeEventListener('pointerdown', onOutside, true);
  window.removeEventListener('blur', onWindowBlur);
});

function onWindowBlur(): void {
  emit('close');
}

function itemClass(item: MenuItem): string {
  if (item.disabled) return 'cursor-default text-deck-faint';
  const tone = item.danger ? 'text-deck-danger' : 'text-deck-text';
  return `cursor-pointer ${tone} hover:bg-deck-hover focus:bg-deck-hover`;
}
</script>

<template>
  <div
    ref="menu"
    role="menu"
    class="fixed z-50 min-w-48 rounded-lg border border-deck-border-strong bg-deck-elevated p-1 font-sans text-[13px] text-deck-text shadow-xl"
    :style="{ left: props.x + 'px', top: props.y + 'px' }"
    @keydown="onKeydown"
    @focusout="onFocusOut"
    @contextmenu.prevent
  >
    <template v-for="item in items" :key="item.key">
      <div v-if="item.separator" role="separator" class="my-1 h-px bg-deck-border" />
      <button
        type="button"
        role="menuitem"
        :data-key="item.key"
        :disabled="item.disabled"
        :aria-haspopup="item.children ? 'menu' : undefined"
        :aria-expanded="item.children ? openKey === item.key : undefined"
        :class="[
          'flex h-7 w-full items-center gap-2 rounded-md px-2 text-left transition-colors duration-150 focus:outline-none focus-visible:ring-2 focus-visible:ring-deck-accent',
          itemClass(item),
          openKey === item.key ? 'bg-deck-hover' : '',
        ]"
        @click="pick(item)"
        @mouseenter="onHover(item)"
      >
        <component :is="item.icon" v-if="item.icon" :size="14" :stroke-width="1.75" class="shrink-0" aria-hidden="true" />
        <span class="flex-1 truncate">{{ item.label }}</span>
        <ChevronRight v-if="item.children" :size="14" :stroke-width="1.75" class="shrink-0 text-deck-faint" aria-hidden="true" />
      </button>
    </template>

    <div
      v-if="openKey"
      ref="sub"
      role="menu"
      data-submenu
      class="fixed z-50 min-w-44 rounded-lg border border-deck-border-strong bg-deck-elevated p-1 shadow-xl"
      :style="subStyle"
    >
      <template v-for="child in items.find((i) => i.key === openKey)?.children ?? []" :key="child.key">
        <div v-if="child.separator" role="separator" class="my-1 h-px bg-deck-border" />
        <button
          type="button"
          role="menuitem"
          :data-key="child.key"
          :disabled="child.disabled"
          :class="[
            'flex h-7 w-full items-center gap-2 rounded-md px-2 text-left transition-colors duration-150 focus:outline-none focus-visible:ring-2 focus-visible:ring-deck-accent',
            itemClass(child),
          ]"
          @click="pick(child)"
        >
          <component :is="child.icon" v-if="child.icon" :size="14" :stroke-width="1.75" class="shrink-0" aria-hidden="true" />
          <span class="truncate">{{ child.label }}</span>
        </button>
      </template>
    </div>
  </div>
</template>

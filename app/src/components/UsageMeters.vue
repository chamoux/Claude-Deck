<script setup lang="ts">
import { Gauge, RefreshCw } from 'lucide-vue-next';
import { computed, onBeforeUnmount, ref, watch } from 'vue';
import { footerLimits, formatAgo, formatDuration, formatResetAt, usageLabel, usageLevel } from '../usage';
import type { UsageLevel, UsageLimit } from '../usage';
import { useDeck } from '../useDeck';

const usage = useDeck().usage;
const root = ref<HTMLElement | null>(null);
const open = ref(false);

const FILL: Record<UsageLevel, string> = {
  normal: 'bg-deck-accent',
  warning: 'bg-dot-needs',
  critical: 'bg-deck-danger',
};
const TEXT: Record<UsageLevel, string> = {
  normal: 'text-deck-text',
  warning: 'text-dot-needs',
  critical: 'text-deck-danger',
};

const limits = computed(() => usage.limits.value ?? []);
const shown = computed(() => footerLimits(limits.value));

const pct = (l: UsageLimit): number => Math.round(l.percent);
const width = (l: UsageLimit): string => `${Math.min(100, Math.max(0, l.percent))}%`;
const key = (l: UsageLimit): string => `${l.kind}:${l.model ?? ''}`;

function resetsIn(l: UsageLimit): string | null {
  return l.resets_at ? formatDuration(Date.parse(l.resets_at) - usage.now.value) : null;
}

const summary = computed(() => {
  if (shown.value.length === 0) return 'Claude usage: unavailable. Show details';
  const parts = shown.value.map((l) => {
    const r = resetsIn(l);
    return `${usageLabel(l)} ${pct(l)}% used${r ? `, resets in ${r}` : ''}`;
  });
  return `Claude usage: ${parts.join('; ')}. Show details`;
});

const updated = computed(() => {
  const at = usage.fetchedAt.value;
  return at === null ? null : `Updated ${formatAgo(usage.now.value - at)}`;
});

function onPointerDown(e: PointerEvent): void {
  if (!root.value?.contains(e.target as Node)) open.value = false;
}
function onKeydown(e: KeyboardEvent): void {
  if (e.key === 'Escape') open.value = false;
}
function unlisten(): void {
  document.removeEventListener('pointerdown', onPointerDown);
  document.removeEventListener('keydown', onKeydown);
}
watch(open, (on) => {
  if (on) {
    document.addEventListener('pointerdown', onPointerDown);
    document.addEventListener('keydown', onKeydown);
  } else {
    unlisten();
  }
});
onBeforeUnmount(unlisten);
</script>

<template>
  <div ref="root" class="relative flex shrink-0 items-center" data-testid="usage">
    <button
      type="button"
      data-testid="usage-toggle"
      aria-haspopup="dialog"
      :aria-expanded="open"
      :aria-label="summary"
      :title="usage.problem.value?.message ?? 'Claude usage: click for details'"
      class="flex h-[22px] cursor-pointer items-center gap-3 rounded-md px-1.5 transition-colors duration-150 hover:bg-deck-hover focus:outline-none focus-visible:ring-2 focus-visible:ring-deck-accent"
      @click="open = !open"
    >
      <template v-if="shown.length > 0">
        <span
          v-for="l in shown"
          :key="key(l)"
          data-testid="usage-meter"
          :class="['flex items-center gap-1.5 transition-opacity duration-150', usage.problem.value ? 'opacity-60' : '']"
        >
          <span class="text-deck-faint">{{ usageLabel(l, true) }}</span>
          <span class="h-1 w-14 overflow-hidden rounded-full bg-deck-border-strong" aria-hidden="true">
            <span
              data-testid="usage-fill"
              :class="[
                'block h-full rounded-full transition-[width] duration-500 ease-out motion-reduce:transition-none',
                FILL[usageLevel(l.percent)],
              ]"
              :style="{ width: width(l) }"
            />
          </span>
          <span data-testid="usage-percent" :class="['font-mono tabular-nums', TEXT[usageLevel(l.percent)]]">{{ pct(l) }}%</span>
          <span v-if="resetsIn(l)" class="font-mono text-deck-faint tabular-nums">{{ resetsIn(l) }}</span>
        </span>
      </template>
      <span v-else class="flex items-center gap-1.5 text-deck-faint" data-testid="usage-empty">
        <Gauge :size="14" :stroke-width="1.75" aria-hidden="true" />
        <span>{{ usage.problem.value ? 'Usage unavailable' : 'Loading usage…' }}</span>
      </span>
    </button>

    <div
      v-if="open"
      role="dialog"
      aria-label="Claude usage"
      data-testid="usage-panel"
      class="absolute right-0 bottom-[calc(100%+6px)] z-50 w-[300px] rounded-lg border border-deck-border-strong bg-deck-elevated p-3 font-sans text-[13px] text-deck-text shadow-xl"
    >
      <header class="mb-3 flex items-center justify-between">
        <h2 class="text-[11px] font-semibold tracking-wider text-deck-faint uppercase">Claude usage</h2>
        <button
          type="button"
          data-testid="usage-refresh"
          aria-label="Refresh usage"
          title="Refresh usage"
          :disabled="usage.loading.value"
          class="flex size-6 cursor-pointer items-center justify-center rounded-md text-deck-muted transition-colors duration-150 hover:bg-deck-hover hover:text-deck-text focus:outline-none focus-visible:ring-2 focus-visible:ring-deck-accent disabled:cursor-default"
          @click="usage.refresh()"
        >
          <RefreshCw
            :size="14"
            :stroke-width="1.75"
            :class="usage.loading.value ? 'animate-spin motion-reduce:animate-none' : ''"
            aria-hidden="true"
          />
        </button>
      </header>

      <ul v-if="limits.length > 0" class="flex flex-col gap-3">
        <li v-for="l in limits" :key="key(l)" data-testid="usage-row">
          <div class="flex items-baseline justify-between gap-3">
            <span class="truncate">{{ usageLabel(l) }}</span>
            <span :class="['shrink-0 font-mono text-xs tabular-nums', TEXT[usageLevel(l.percent)]]">{{ pct(l) }}% used</span>
          </div>
          <div
            role="meter"
            aria-valuemin="0"
            aria-valuemax="100"
            :aria-valuenow="pct(l)"
            :aria-label="usageLabel(l)"
            class="mt-1.5 h-1.5 overflow-hidden rounded-full bg-deck-border-strong"
          >
            <div
              :class="[
                'h-full rounded-full transition-[width] duration-500 ease-out motion-reduce:transition-none',
                FILL[usageLevel(l.percent)],
              ]"
              :style="{ width: width(l) }"
            />
          </div>
          <p v-if="l.resets_at" class="mt-1 text-xs text-deck-faint">
            Resets in {{ resetsIn(l) }} · {{ formatResetAt(l.resets_at, usage.now.value) }}
          </p>
        </li>
      </ul>

      <p
        v-if="usage.problem.value"
        data-testid="usage-problem"
        :class="['text-xs', limits.length > 0 ? 'mt-3' : '', usage.problem.value.kind === 'error' ? 'text-deck-danger' : 'text-deck-muted']"
      >
        {{ usage.problem.value.message }}
      </p>
      <p v-else-if="limits.length === 0" class="text-xs text-deck-muted">Loading usage…</p>

      <p class="mt-3 border-t border-deck-border pt-2 text-xs text-deck-faint">
        <span v-if="updated" data-testid="usage-updated">{{ updated }} · </span>Refreshes every minute
      </p>
    </div>
  </div>
</template>

<script setup lang="ts">
import { effectiveMode, useAppStore, type ThemeMode } from '@/stores/app'
import { useRouter } from 'vue-router'

const store = useAppStore()
const router = useRouter()

// The theme toggle is a tri-state: light / system / dark. `SelectButton`
// needs concrete values to bind, so we model it on the stored mode.
const themeOptions = [
  { label: '', value: 'light', icon: 'pi pi-sun' },
  { label: '', value: 'system', icon: 'pi pi-desktop' },
  { label: '', value: 'dark', icon: 'pi pi-moon' },
] as const

// Tooltip describing what the current effective mode resolves to.
const resolvedLabel = computed(
  () => `Theme: ${effectiveMode(store.themeMode)} (click to change)`,
)

function goSettings() {
  router.push('/settings')
}
</script>

<template>
  <header
    class="h-12 shrink-0 flex items-center justify-between px-4 border-b border-solid"
    style="border-color: var(--app-border); background: var(--app-surface)"
  >
    <!-- Wordmark -->
    <div class="flex items-center gap-2 select-none">
      <span
        class="inline-flex items-center justify-center rounded-xl text-white"
        style="background: var(--p-primary-500); width: 26px; height: 26px"
      >
        <!-- Minimal cat glyph -->
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" aria-hidden="true">
          <path
            d="M4 6l2 3a8 8 0 0 1 12 0l2-3v6a6 6 0 0 1-6 6h-4a6 6 0 0 1-6-6V6z"
            fill="currentColor"
            opacity="0.95"
          />
          <circle cx="9.5" cy="11" r="1.1" fill="#0b1120" />
          <circle cx="14.5" cy="11" r="1.1" fill="#0b1120" />
        </svg>
      </span>
      <span class="font-semibold text-base tracking-tight">Nekocode</span>
    </div>

    <!-- Actions -->
    <div class="flex items-center gap-1">
      <SelectButton
        :modelValue="store.themeMode"
        :options="themeOptions as unknown as { value: ThemeMode }[]"
        optionValue="value"
        :allowEmpty="false"
        size="small"
        :title="resolvedLabel"
        @update:modelValue="store.setThemeMode($event as ThemeMode)"
      >
        <template #option="{ option }">
          <i :class="(option as { icon: string }).icon" />
        </template>
      </SelectButton>
      <Button
        icon="pi pi-cog"
        variant="text"
        severity="secondary"
        rounded
        text
        title="Settings"
        aria-label="Settings"
        @click="goSettings"
      />
    </div>
  </header>
</template>

<style scoped>
/* Tighten the tri-state toggle so it sits neatly in the 48px bar. */
:deep(.p-selectbutton .p-button) {
  padding: 0.4rem 0.6rem;
}
</style>

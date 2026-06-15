<script setup lang="ts">
import { effectiveMode, useAppStore, type ThemeMode } from '@/stores/app'

const store = useAppStore()
const themeOptions: { label: string; value: ThemeMode; icon: string; desc: string }[] = [
  { label: 'Light', value: 'light', icon: 'pi pi-sun', desc: 'Always use the light theme.' },
  {
    label: 'System',
    value: 'system',
    icon: 'pi pi-desktop',
    desc: 'Follow your operating system setting.',
  },
  { label: 'Dark', value: 'dark', icon: 'pi pi-moon', desc: 'Always use the dark theme.' },
]
</script>

<template>
  <div class="h-full overflow-auto">
    <div class="mx-auto max-w-2xl px-6 py-8">
      <h1 class="text-2xl font-semibold tracking-tight mb-1">Settings</h1>
      <p class="text-sm mb-6" style="color: var(--app-text-muted)">
        Personalize your Nekocode experience.
      </p>

      <section class="card">
        <h2 class="card-title">Appearance</h2>
        <p class="card-subtitle">
          Effective theme: <strong>{{ effectiveMode(store.themeMode) }}</strong>
        </p>
        <div class="grid grid-cols-1 sm:grid-cols-3 gap-3 mt-3">
          <button
            v-for="opt in themeOptions"
            :key="opt.value"
            type="button"
            class="theme-option"
            :class="{ active: store.themeMode === opt.value }"
            @click="store.setThemeMode(opt.value)"
          >
            <i :class="opt.icon" class="text-lg"></i>
            <span class="font-medium">{{ opt.label }}</span>
            <span class="theme-option-desc">{{ opt.desc }}</span>
          </button>
        </div>
      </section>

      <section class="card mt-5">
        <h2 class="card-title">More</h2>
        <p class="card-subtitle">
          Account, providers, and other configuration will appear here in a future update.
        </p>
      </section>
    </div>
  </div>
</template>

<style scoped>
.card {
  background: var(--app-surface);
  border: 1px solid var(--app-border);
  border-radius: 14px;
  padding: 18px 20px;
}
.card-title {
  font-size: 1.05rem;
  font-weight: 600;
  margin: 0 0 4px;
}
.card-subtitle {
  font-size: 0.85rem;
  color: var(--app-text-muted);
  margin: 0;
}
.theme-option {
  display: flex;
  flex-direction: column;
  align-items: flex-start;
  gap: 4px;
  text-align: left;
  padding: 14px;
  border-radius: 12px;
  border: 1px solid var(--app-border);
  background: var(--app-surface);
  cursor: pointer;
  transition:
    border-color 0.15s ease,
    background-color 0.15s ease;
}
.theme-option:hover {
  background: var(--p-surface-100);
}
.app-dark .theme-option:hover {
  background: var(--p-surface-800);
}
.theme-option.active {
  border-color: var(--p-primary-500);
  background: color-mix(in srgb, var(--p-primary-500) 10%, transparent);
}
.theme-option i {
  color: var(--p-primary-500);
}
.theme-option-desc {
  font-size: 0.75rem;
  color: var(--app-text-muted);
}
</style>

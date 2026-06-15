<script setup lang="ts">
import { useRouter } from 'vue-router'
import AppearanceSection from './sections/AppearanceSection.vue'

const router = useRouter()

// Settings sections shown in the left nav. New sections are added by appending
// here plus a `currentComponent` branch — the shell handles switching.
const sections = [{ key: 'appearance', label: 'Appearance', icon: 'pi pi-sliders-h' }] as const

const active = ref<(typeof sections)[number]['key']>('appearance')

const currentComponent = computed(() => {
  switch (active.value) {
    case 'appearance':
      return AppearanceSection
    default:
      return null
  }
})
</script>

<template>
  <div class="h-full grid grid-cols-[220px_1fr] overflow-hidden">
    <!-- Nav -->
    <aside class="settings-nav">
      <button class="back-btn" type="button" title="Back" @click="router.push('/')">
        <i class="pi pi-arrow-left" />
        <span>Back</span>
      </button>
      <div class="nav-header">Settings</div>
      <nav class="nav-list">
        <button
          v-for="s in sections"
          :key="s.key"
          type="button"
          class="nav-item"
          :class="{ active: active === s.key }"
          @click="active = s.key"
        >
          <i :class="s.icon" />
          <span>{{ s.label }}</span>
        </button>
      </nav>
    </aside>

    <!-- Content -->
    <main class="settings-content overflow-auto">
      <div class="mx-auto max-w-2xl px-6 py-8">
        <component :is="currentComponent" v-if="currentComponent" />
      </div>
    </main>
  </div>
</template>

<style scoped>
.settings-nav {
  display: flex;
  flex-direction: column;
  background: var(--app-surface);
  border-right: 1px solid var(--app-border);
}
.back-btn {
  display: flex;
  align-items: center;
  gap: 8px;
  margin: 12px 8px 4px;
  padding: 8px 12px;
  border: 1px solid transparent;
  border-radius: 10px;
  background: transparent;
  color: var(--app-text-muted);
  font-size: 0.9rem;
  text-align: left;
  cursor: pointer;
  transition: background-color 0.12s ease;
}
.back-btn:hover {
  background: var(--p-surface-100);
  color: var(--app-text);
}
.app-dark .back-btn:hover {
  background: var(--p-surface-800);
}
.nav-header {
  padding: 18px 18px 10px;
  font-size: 0.75rem;
  font-weight: 600;
  letter-spacing: 0.04em;
  text-transform: uppercase;
  color: var(--app-text-muted);
}
.nav-list {
  display: flex;
  flex-direction: column;
  padding: 4px 8px;
}
.nav-item {
  display: flex;
  align-items: center;
  gap: 10px;
  margin: 2px 0;
  padding: 8px 12px;
  border: 1px solid transparent;
  border-radius: 10px;
  background: transparent;
  color: var(--app-text);
  font-size: 0.9rem;
  text-align: left;
  cursor: pointer;
  transition: background-color 0.12s ease;
}
.nav-item:hover {
  background: var(--p-surface-100);
}
.app-dark .nav-item:hover {
  background: var(--p-surface-800);
}
.nav-item.active {
  background: color-mix(in srgb, var(--p-primary-500) 14%, transparent);
}
.nav-item.active i,
.nav-item.active span {
  color: var(--p-primary-500);
  font-weight: 500;
}
</style>

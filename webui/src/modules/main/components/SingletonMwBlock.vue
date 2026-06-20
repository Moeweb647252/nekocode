<script setup lang="ts">
defineProps<{
  icon: string
  name: string
  enabled: boolean
  expanded: boolean
}>()

defineEmits<{
  'update:enabled': [value: boolean]
  'update:expanded': [value: boolean]
}>()
</script>

<template>
  <div class="mw-block">
    <div class="mw-header" @click="$emit('update:expanded', !expanded)">
      <i :class="['pi', 'mw-icon', icon]"></i>
      <span class="mw-name">{{ name }}</span>
      <span class="mw-status" :class="{ on: enabled }">
        {{ enabled ? 'Enabled' : 'Disabled' }}
      </span>
      <div class="mw-toggle" @click.stop>
        <ToggleSwitch
          :model-value="enabled"
          @update:model-value="$emit('update:enabled', $event)"
        />
      </div>
      <i
        class="pi mw-chevron"
        :class="expanded ? 'pi-chevron-up' : 'pi-chevron-down'"
      ></i>
    </div>
    <div v-show="expanded" class="mw-body">
      <slot />
    </div>
  </div>
</template>

<style scoped>
.mw-header {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 10px 4px;
  cursor: pointer;
  border-radius: 8px;
  transition: background-color 0.12s ease;
}
.mw-header:hover {
  background: var(--p-surface-100);
}
:global(.app-dark) .mw-header:hover {
  background: var(--p-surface-800);
}
.mw-icon {
  font-size: 0.95rem;
  color: var(--p-primary-500);
  width: 18px;
  text-align: center;
}
.mw-name {
  font-size: 0.88rem;
  font-weight: 600;
}
.mw-status {
  font-size: 0.72rem;
  color: var(--app-text-muted);
  padding: 2px 8px;
  border-radius: 9999px;
  background: var(--p-surface-100);
}
:global(.app-dark) .mw-status {
  background: var(--p-surface-800);
}
.mw-status.on {
  color: var(--p-primary-700);
  background: color-mix(in srgb, var(--p-primary-500) 14%, transparent);
}
:global(.app-dark) .mw-status.on {
  color: var(--p-primary-400);
}
.mw-toggle {
  flex-shrink: 0;
  display: inline-flex;
  align-items: center;
}
.mw-chevron {
  margin-left: auto;
  font-size: 0.8rem;
  color: var(--app-text-muted);
}
.mw-body {
  padding: 4px 4px 8px 28px;
}
</style>
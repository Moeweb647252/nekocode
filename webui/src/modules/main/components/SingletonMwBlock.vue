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

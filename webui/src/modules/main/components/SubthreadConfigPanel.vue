<script setup lang="ts">
import type { SingletonEntry } from './ThreadSettingsDialog.vue'
import type { SubthreadConfig } from '@/api/types'

defineProps<{
  entry: SingletonEntry<SubthreadConfig>
}>()
</script>

<template>
  <div class="field">
    <label class="field-label">Allow subthreads</label>
    <ToggleSwitch
      :model-value="entry.config.allowSubthread ?? false"
      @update:model-value="(v) => (entry.config.allowSubthread = v)"
    />
    <span class="field-hint">
      When enabled, spawned subthreads will also receive the subthread middleware,
      allowing them to spawn their own subthreads (recursive fan-out). Disable to
      limit the thread tree to a single level of nesting.
    </span>
  </div>
</template>
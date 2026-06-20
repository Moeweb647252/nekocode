<script setup lang="ts">
import type { SingletonEntry } from './ThreadSettingsDialog.vue'
import type { FileConfig } from '@/api/types'

defineProps<{
  entry: SingletonEntry<FileConfig>
}>()
</script>

<template>
  <div class="field">
    <label class="field-label">Working directory</label>
    <InputText
      v-model="entry.config.workingDirectory"
      class="field-input"
      placeholder="(inherit server cwd)"
    />
  </div>
</template>
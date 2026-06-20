<script setup lang="ts">
import type { SingletonEntry } from './ThreadSettingsDialog.vue'
import type { ShellConfig } from '@/api/types'

defineProps<{
  entry: SingletonEntry<ShellConfig>
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
  <div class="field">
    <label class="field-label">Shell</label>
    <InputText v-model="entry.config.shell" class="field-input" placeholder="bash" />
  </div>
  <div class="field">
    <label class="field-label">Timeout (seconds)</label>
    <InputText
      :model-value="
        entry.config.timeoutSecs == null ? '' : String(entry.config.timeoutSecs)
      "
      @update:model-value="
        (v: string | undefined) =>
          (entry.config.timeoutSecs =
            v === '' || v === undefined ? null : Number(v))
      "
      class="field-input"
      placeholder="(no timeout)"
    />
  </div>
  <div class="field">
    <label class="field-label">Environment variables</label>
    <div class="env-list">
      <div v-for="(row, i) in entry.envsRows" :key="i" class="env-row">
        <InputText v-model="row.key" class="env-key" placeholder="KEY" />
        <InputText v-model="row.value" class="env-val" placeholder="value" />
        <button
          type="button"
          class="env-remove"
          title="Remove"
          @click="entry.envsRows.splice(i, 1)"
        >
          <i class="pi pi-times"></i>
        </button>
      </div>
      <button type="button" class="env-add" @click="entry.envsRows.push({ key: '', value: '' })">
        <i class="pi pi-plus"></i> Add variable
      </button>
    </div>
  </div>
</template>
<script setup lang="ts">
import type { SingletonEntry } from './ThreadSettingsDialog.vue'
import type { SkillsConfig } from '@/api/types'

defineProps<{
  entry: SingletonEntry<SkillsConfig>
  availableSkills: { name: string; description: string }[]
}>()
</script>

<template>
  <div class="field">
    <label class="field-label">Enabled skills</label>
    <MultiSelect
      :model-value="entry.config.enabled || []"
      :options="availableSkills"
      option-label="name"
      option-value="name"
      display="chip"
      placeholder="Select skills to enable"
      class="field-input"
      @update:model-value="(v) => (entry.config.enabled = v)"
    >
      <template #option="{ option }">
        <div class="skill-option">
          <span class="skill-option-name">{{ option.name }}</span>
          <span v-if="option.description" class="skill-option-desc">
            {{ option.description }}
          </span>
        </div>
      </template>
    </MultiSelect>
    <span class="field-hint">
      Skills inject behavioral prompts into the system prompt. Built-in and
      user-defined skills are listed.
    </span>
  </div>
</template>
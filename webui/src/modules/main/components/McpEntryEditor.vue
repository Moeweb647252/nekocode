<script setup lang="ts">
import type { McpEntry } from './ThreadSettingsDialog.vue'

defineProps<{
  entry: McpEntry
  index: number
  probing: boolean
  probeError: string
  transportOptions: { label: string; value: string; icon: string }[]
}>()

const emit = defineEmits<{
  'test-connection': [entry: McpEntry]
  'remove': [index: number]
}>()

function addEnvRow(entry: McpEntry) {
  entry.envsRows.push({ key: '', value: '' })
}
function removeEnvRow(entry: McpEntry, index: number) {
  entry.envsRows.splice(index, 1)
}
function addAuthHeaderRow(entry: McpEntry) {
  entry.authHeadersRows.push({ key: '', value: '' })
}
function removeAuthHeaderRow(entry: McpEntry, index: number) {
  entry.authHeadersRows.splice(index, 1)
}
</script>

<template>
  <div class="mcp-item">
    <div class="mcp-item-header">
      <span class="mcp-item-label">
        {{ entry.config.serverCommand || entry.config.serverUrl || 'MCP Server' }}
      </span>
      <div class="mcp-item-actions">
        <ToggleSwitch v-model="entry.enabled" />
        <button
          type="button"
          class="mcp-item-delete"
          title="Remove"
          @click="emit('remove', index)"
        >
          <i class="pi pi-trash"></i>
        </button>
      </div>
    </div>
    <div class="mcp-item-body">
      <!-- Transport selector -->
      <div class="field">
        <label class="field-label">Transport</label>
        <SelectButton
          :model-value="entry.config.transport || 'stdio'"
          :options="transportOptions"
          option-value="value"
          option-label="label"
          @update:model-value="(v) => (entry.config.transport = v)"
        >
          <template #option="{ option }">
            <i class="pi" :class="option.icon"></i>
            <span class="ml-1">{{ option.label }}</span>
          </template>
        </SelectButton>
      </div>

      <!-- HTTP transport -->
      <template v-if="(entry.config.transport || 'stdio') === 'http'">
        <div class="field">
          <label class="field-label">Server URL</label>
          <InputText
            v-model="entry.config.serverUrl"
            class="field-input"
            placeholder="http://localhost:8080/mcp"
          />
          <span class="field-hint">Streamable HTTP endpoint for the MCP server.</span>
        </div>
        <div class="field">
          <label class="field-label">Auth Headers</label>
          <div class="env-list">
            <div v-for="(row, i) in entry.authHeadersRows" :key="i" class="env-row">
              <InputText v-model="row.key" class="env-key" placeholder="Header-Name" />
              <InputText v-model="row.value" class="env-val" placeholder="value" />
              <button
                type="button"
                class="env-remove"
                title="Remove"
                @click="removeAuthHeaderRow(entry, i)"
              >
                <i class="pi pi-times"></i>
              </button>
            </div>
            <button type="button" class="env-add" @click="addAuthHeaderRow(entry)">
              <i class="pi pi-plus"></i> Add header
            </button>
          </div>
          <span class="field-hint">
            Custom HTTP headers sent with every request (e.g. Authorization, X-API-Key).
          </span>
        </div>
      </template>

      <!-- Stdio transport -->
      <template v-else>
        <div class="field">
          <label class="field-label">Server command</label>
          <InputText
            v-model="entry.config.serverCommand"
            class="field-input"
            placeholder="npx -y @modelcontextprotocol/server-filesystem"
          />
          <span class="field-hint">
            Shell command to spawn the MCP server over stdio.
          </span>
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
                @click="removeEnvRow(entry, i)"
              >
                <i class="pi pi-times"></i>
              </button>
            </div>
            <button type="button" class="env-add" @click="addEnvRow(entry)">
              <i class="pi pi-plus"></i> Add variable
            </button>
          </div>
        </div>
      </template>

      <!-- Tools -->
      <div class="field">
        <div class="field-row">
          <label class="field-label">Tools</label>
          <Button
            label="Test connection"
            icon="pi pi-bolt"
            size="small"
            severity="secondary"
            :loading="probing"
            @click="emit('test-connection', entry)"
          />
        </div>
        <span class="field-hint">
          Tools are discovered from the server — click "Test connection" to probe.
          Toggle which ones the model can use.
        </span>
        <div v-if="probeError" class="state-error">{{ probeError }}</div>
        <div class="env-list">
          <div v-for="(row, i) in entry.toolsRows" :key="i" class="tool-row">
            <div class="tool-info">
              <span class="tool-name-display">{{ row.name }}</span>
              <span v-if="row.description" class="tool-desc">{{ row.description }}</span>
            </div>
            <ToggleSwitch v-model="row.enabled" />
          </div>
          <div v-if="!entry.toolsRows.length" class="mw-empty-hint">
            No tools discovered yet.
          </div>
        </div>
      </div>
    </div>
  </div>
</template>
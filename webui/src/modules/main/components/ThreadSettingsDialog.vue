<script setup lang="ts">
import { computed } from 'vue'
import {
  activateThread,
  createMiddleware,
  deleteMiddleware,
  getModels,
  getThread,
  listMiddlewares,
  listSkills,
  probeMcp,
  updateMiddleware,
  updateThread,
} from '@/api'
import type { DynamicDialogInstance } from 'primevue/dynamicdialogoptions'
import type { ShellConfig, FileConfig, SkillsConfig, SubthreadConfig, McpConfig, MiddlewareConfig } from '@/api/types'
import SingletonMwBlock from './SingletonMwBlock.vue'
import ShellConfigPanel from './ShellConfigPanel.vue'
import FileConfigPanel from './FileConfigPanel.vue'
import SkillsConfigPanel from './SkillsConfigPanel.vue'
import SubthreadConfigPanel from './SubthreadConfigPanel.vue'
import McpEntryEditor from './McpEntryEditor.vue'

// ── Public types (shared with child components) ──

export interface SingletonEntry<T = ShellConfig | FileConfig | SkillsConfig | SubthreadConfig> {
  id: number | null
  enabled: boolean
  originalEnabled: boolean
  config: T
  original: T
  envsRows: { key: string; value: string }[]
}

export interface McpEntry {
  id: number | null
  enabled: boolean
  originalEnabled: boolean
  config: McpConfig
  original: McpConfig
  envsRows: { key: string; value: string }[]
  authHeadersRows: { key: string; value: string }[]
  toolsRows: { name: string; description: string; enabled: boolean }[]
}

// ── State ──

const dialogRef = inject<Ref<DynamicDialogInstance>>('dialogRef')
const threadId = (dialogRef?.value?.data as { threadId?: number } | undefined)?.threadId

const models = ref<string[]>([])
const loading = ref(true)
const saving = ref(false)
const error = ref('')

const title = ref('')
const originalTitle = ref('')
const model = ref('')
const originalModel = ref('')

const activeSection = ref<'basic' | 'middlewares'>('basic')

const shellEntry = ref<SingletonEntry<ShellConfig> | null>(null)
const toolEntry = ref<SingletonEntry<FileConfig> | null>(null)
const skillsEntry = ref<SingletonEntry<SkillsConfig> | null>(null)
const subthreadEntry = ref<SingletonEntry<SubthreadConfig> | null>(null)
const shellExpanded = ref(false)
const toolExpanded = ref(false)
const skillsExpanded = ref(false)
const subthreadExpanded = ref(false)
const mcpExpanded = ref(false)

const availableSkills = ref<{ name: string; description: string }[]>([])

const mcpEntries = ref<McpEntry[]>([])
const deletedMcpIds = ref<number[]>([])

const probing = ref(false)
const probeError = ref('')

// ── Computed ──

const modelChanged = computed(() => model.value !== originalModel.value)
const titleChanged = computed(() => title.value !== originalTitle.value)

function singletonChanged<T>(e: SingletonEntry<T>): boolean {
  return e.enabled !== e.originalEnabled
}

function mcpChanged(e: McpEntry): boolean {
  return e.enabled !== e.originalEnabled
}

const middlewareChanged = computed(() => {
  if (shellEntry.value && singletonChanged(shellEntry.value)) return true
  if (toolEntry.value && singletonChanged(toolEntry.value)) return true
  if (skillsEntry.value && singletonChanged(skillsEntry.value)) return true
  if (subthreadEntry.value && singletonChanged(subthreadEntry.value)) return true
  for (const e of mcpEntries.value) {
    if (e.id == null || mcpChanged(e)) return true
  }
  if (deletedMcpIds.value.length > 0) return true
  return false
})

const dirty = computed(() => titleChanged.value || modelChanged.value || middlewareChanged.value)

// ── Helpers ──

function splitEnvs(cfg: { envs?: Record<string, string> }): { key: string; value: string }[] {
  const envs = cfg.envs ?? {}
  return Object.entries(envs).map(([key, value]) => ({ key, value: String(value ?? '') }))
}

function splitAuthHeaders(
  cfg: { authHeaders?: Record<string, string> },
): { key: string; value: string }[] {
  const headers = cfg.authHeaders ?? {}
  return Object.entries(headers).map(([key, value]) => ({ key, value: String(value ?? '') }))
}

function splitTools(
  cfg: { toolsEnabled?: Record<string, boolean> },
): { name: string; description: string; enabled: boolean }[] {
  const tools = cfg.toolsEnabled ?? {}
  return Object.entries(tools).map(([name, on]) => ({ name, description: '', enabled: !!on }))
}

function defaultMcpConfig(): McpConfig {
  return { transport: 'stdio', serverCommand: '', serverUrl: '', envs: {}, authHeaders: {}, toolsEnabled: {} }
}

const TRANSPORT_OPTIONS: { label: string; value: string; icon: string }[] = [
  { label: 'Stdio', value: 'stdio', icon: 'pi-terminal' },
  { label: 'HTTP', value: 'http', icon: 'pi-globe' },
]

// ── Lifecycle ──

onMounted(async () => {
  if (threadId == null) return
  try {
    const [thread, mws, modelList, skills] = await Promise.all([
      getThread(threadId),
      listMiddlewares(threadId),
      getModels(),
      listSkills(),
    ])
    title.value = thread.title ?? ''
    originalTitle.value = title.value
    model.value = thread.model ?? ''
    originalModel.value = model.value
    models.value = modelList
    availableSkills.value = skills.map((s) => ({ name: s.name, description: s.description ?? '' }))

    for (const m of mws) {
      if (m.name === 'shell' && !shellEntry.value) {
        shellEntry.value = {
          id: m.id,
          enabled: m.enabled,
          originalEnabled: m.enabled,
          config: { ...m.config } as ShellConfig,
          original: { ...m.config } as ShellConfig,
          envsRows: splitEnvs(m.config as ShellConfig),
        }
      } else if (m.name === 'tool' && !toolEntry.value) {
        toolEntry.value = {
          id: m.id,
          enabled: m.enabled,
          originalEnabled: m.enabled,
          config: { ...m.config } as FileConfig,
          original: { ...m.config } as FileConfig,
          envsRows: [],
        }
      } else if (m.name === 'skills' && !skillsEntry.value) {
        skillsEntry.value = {
          id: m.id,
          enabled: m.enabled,
          originalEnabled: m.enabled,
          config: { ...m.config } as SkillsConfig,
          original: { ...m.config } as SkillsConfig,
          envsRows: [],
        }
      } else if (m.name === 'subthread' && !subthreadEntry.value) {
        subthreadEntry.value = {
          id: m.id,
          enabled: m.enabled,
          originalEnabled: m.enabled,
          config: { ...m.config } as SubthreadConfig,
          original: { ...m.config } as SubthreadConfig,
          envsRows: [],
        }
      } else if (m.name === 'mcp') {
        mcpEntries.value.push({
          id: m.id,
          enabled: m.enabled,
          originalEnabled: m.enabled,
          config: { ...m.config } as McpConfig,
          original: { ...m.config } as McpConfig,
          envsRows: splitEnvs(m.config as McpConfig),
          authHeadersRows: splitAuthHeaders(m.config as McpConfig),
          toolsRows: splitTools(m.config as McpConfig),
        })
      }
    }

    if (!skillsEntry.value) {
      const defaultCfg: SkillsConfig = { enabled: [] }
      skillsEntry.value = { id: null, enabled: false, originalEnabled: false, config: { ...defaultCfg }, original: { ...defaultCfg }, envsRows: [] }
    }
    if (!subthreadEntry.value) {
      const defaultCfg: SubthreadConfig = { allowSubthread: false }
      subthreadEntry.value = { id: null, enabled: false, originalEnabled: false, config: { ...defaultCfg }, original: { ...defaultCfg }, envsRows: [] }
    }
  } catch (e) {
    error.value = e instanceof Error ? e.message : String(e)
  } finally {
    loading.value = false
  }
})

// ── Methods ──

async function testConnection(entry: McpEntry) {
  probing.value = true
  probeError.value = ''
  const { transport, serverCommand, serverUrl, envs, authHeaders } = entry.config
  try {
    const tools = await probeMcp(transport, serverCommand ?? null, serverUrl ?? null, envs, authHeaders)
    const prev = new Map(entry.toolsRows.map((r) => [r.name, r.enabled]))
    entry.toolsRows = tools.map((t) => ({
      name: t.name,
      description: t.description ?? '',
      enabled: prev.has(t.name) ? prev.get(t.name)! : true,
    }))
  } catch (e) {
    probeError.value = e instanceof Error ? e.message : String(e)
  } finally {
    probing.value = false
  }
}

function addMcpEntry() {
  mcpEntries.value.push({
    id: null,
    enabled: true,
    originalEnabled: true,
    config: defaultMcpConfig(),
    original: defaultMcpConfig(),
    envsRows: [],
    authHeadersRows: [],
    toolsRows: [],
  })
}

function removeMcpEntry(index: number) {
  const e = mcpEntries.value[index]
  if (e && e.id != null) deletedMcpIds.value.push(e.id)
  mcpEntries.value.splice(index, 1)
}

function flushEnvs(entry: { envsRows: { key: string; value: string }[]; config: { envs: Record<string, string> } }) {
  const envs: Record<string, string> = {}
  for (const row of entry.envsRows) {
    const key = row.key.trim()
    if (key) envs[key] = row.value
  }
  entry.config.envs = envs
}

function flushAuthHeaders(entry: { authHeadersRows: { key: string; value: string }[]; config: { authHeaders: Record<string, string> } }) {
  const headers: Record<string, string> = {}
  for (const row of entry.authHeadersRows) {
    const key = row.key.trim()
    if (key) headers[key] = row.value
  }
  entry.config.authHeaders = headers
}

function flushTools(entry: McpEntry) {
  const toolsEnabled: Record<string, boolean> = {}
  for (const row of entry.toolsRows) {
    const name = row.name.trim()
    if (name) toolsEnabled[name] = row.enabled
  }
  entry.config.toolsEnabled = toolsEnabled
}

async function saveSingleton<T extends MiddlewareConfig>(entry: SingletonEntry<T>, name: string) {
  if (entry.id == null) {
    await createMiddleware(threadId!, name, entry.config)
  } else if (singletonChanged(entry)) {
    await updateMiddleware(entry.id, entry.config, entry.enabled)
  }
}

async function save() {
  if (threadId == null || saving.value || !dirty.value) {
    dialogRef?.value?.close(false)
    return
  }
  saving.value = true
  error.value = ''
  try {
    if (titleChanged.value) await updateThread(threadId, title.value, null)
    if (modelChanged.value) await updateThread(threadId, null, model.value)
    let needsReactivation = modelChanged.value

    if (shellEntry.value) { flushEnvs(shellEntry.value); await saveSingleton(shellEntry.value, 'shell') }
    if (toolEntry.value) await saveSingleton(toolEntry.value, 'tool')
    if (skillsEntry.value) await saveSingleton(skillsEntry.value, 'skills')
    if (subthreadEntry.value) await saveSingleton(subthreadEntry.value, 'subthread')

    for (const e of mcpEntries.value) {
      flushEnvs(e)
      flushAuthHeaders(e)
      flushTools(e)
      if (e.id == null) {
        await createMiddleware(threadId, 'mcp', e.config)
        needsReactivation = true
      } else if (mcpChanged(e)) {
        await updateMiddleware(e.id, e.config, e.enabled)
        needsReactivation = true
      }
    }

    for (const id of deletedMcpIds.value) {
      await deleteMiddleware(id)
      needsReactivation = true
    }

    if (needsReactivation) await activateThread(threadId)
    dialogRef?.value?.close(true)
  } catch (e) {
    error.value = e instanceof Error ? e.message : String(e)
  } finally {
    saving.value = false
  }
}

function cancel() {
  dialogRef?.value?.close(false)
}
</script>

<template>
  <div class="settings-dialog">
    <nav class="sidebar">
      <button type="button" class="nav-item" :class="{ active: activeSection === 'basic' }" @click="activeSection = 'basic'">
        <i class="pi pi-info-circle nav-icon"></i>
        <span class="nav-label">Basic Info</span>
      </button>
      <button type="button" class="nav-item" :class="{ active: activeSection === 'middlewares' }" @click="activeSection = 'middlewares'">
        <i class="pi pi-sliders-h nav-icon"></i>
        <span class="nav-label">Middlewares</span>
      </button>
    </nav>

    <main class="content">
      <div v-if="loading" class="state-msg">Loading…</div>
      <div v-else-if="error && !shellEntry && !toolEntry && !mcpEntries.length" class="state-msg state-error">
        {{ error }}
      </div>
      <template v-else>
        <!-- Basic info section -->
        <section v-show="activeSection === 'basic'" class="section">
          <h2 class="section-title">Basic Info</h2>
          <p class="section-subtitle">Title and model for this thread.</p>
          <div class="field">
            <label class="field-label">Title</label>
            <InputText v-model="title" class="field-input" placeholder="Untitled" />
          </div>
          <div class="field">
            <label class="field-label">Model</label>
            <Select v-model="model" :options="models" placeholder="Select a model" class="field-input" />
          </div>
        </section>

        <!-- Middlewares section -->
        <section v-show="activeSection === 'middlewares'" class="section">
          <h2 class="section-title">Middlewares</h2>
          <p class="section-subtitle">Enable and configure per-thread middleware.</p>

          <SingletonMwBlock
            icon="pi-terminal"
            name="Shell"
            :enabled="shellEntry?.enabled ?? false"
            :expanded="shellExpanded"
            @update:enabled="(v) => { if (shellEntry) shellEntry.enabled = v }"
            @update:expanded="shellExpanded = $event"
          >
            <ShellConfigPanel v-if="shellEntry" :entry="shellEntry" />
          </SingletonMwBlock>

          <SingletonMwBlock
            icon="pi-wrench"
            name="Tool"
            :enabled="toolEntry?.enabled ?? false"
            :expanded="toolExpanded"
            @update:enabled="(v) => { if (toolEntry) toolEntry.enabled = v }"
            @update:expanded="toolExpanded = $event"
          >
            <FileConfigPanel v-if="toolEntry" :entry="toolEntry" />
          </SingletonMwBlock>

          <SingletonMwBlock
            icon="pi-star"
            name="Skills"
            :enabled="skillsEntry?.enabled ?? false"
            :expanded="skillsExpanded"
            @update:enabled="(v) => { if (skillsEntry) skillsEntry.enabled = v }"
            @update:expanded="skillsExpanded = $event"
          >
            <SkillsConfigPanel v-if="skillsEntry" :entry="skillsEntry" :available-skills="availableSkills" />
          </SingletonMwBlock>

          <SingletonMwBlock
            icon="pi-sitemap"
            name="Subthread"
            :enabled="subthreadEntry?.enabled ?? false"
            :expanded="subthreadExpanded"
            @update:enabled="(v) => { if (subthreadEntry) subthreadEntry.enabled = v }"
            @update:expanded="subthreadExpanded = $event"
          >
            <SubthreadConfigPanel v-if="subthreadEntry" :entry="subthreadEntry" />
          </SingletonMwBlock>

          <!-- MCP (0..n) -->
          <div class="mw-block">
            <div class="mw-header" @click="mcpExpanded = !mcpExpanded">
              <i class="pi mw-icon pi-bolt"></i>
              <span class="mw-name">MCP Servers</span>
              <Button label="Add" icon="pi pi-plus" size="small" severity="secondary" class="mw-add-btn" @click.stop="addMcpEntry" />
              <i class="pi mw-chevron" :class="mcpExpanded ? 'pi-chevron-up' : 'pi-chevron-down'"></i>
            </div>
            <div v-show="mcpExpanded" class="mw-body">
              <div v-if="!mcpEntries.length" class="mw-empty-hint">No MCP servers configured.</div>
              <McpEntryEditor
                v-for="(entry, idx) in mcpEntries"
                :key="entry.id ?? `new-${idx}`"
                :entry="entry"
                :index="idx"
                :probing="probing"
                :probe-error="probeError"
                :transport-options="TRANSPORT_OPTIONS"
                @test-connection="testConnection"
                @remove="removeMcpEntry"
              />
            </div>
          </div>
        </section>

        <div v-if="error" class="state-error mt-3">{{ error }}</div>

        <div class="actions">
          <Button label="Cancel" severity="secondary" variant="text" :disabled="saving" @click="cancel" />
          <Button label="Save" :disabled="!dirty || saving" :loading="saving" @click="save" />
        </div>
      </template>
    </main>
  </div>
</template>

<style scoped>
.settings-dialog {
  display: grid;
  grid-template-columns: 180px 1fr;
  min-width: 640px;
  max-width: 800px;
  height: 480px;
}

/* Sidebar nav */
.sidebar {
  background: var(--app-surface);
  border-right: 1px solid var(--app-border);
  padding: 12px 8px;
}
.nav-item {
  display: flex;
  align-items: center;
  gap: 10px;
  width: 100%;
  padding: 10px 12px;
  border-radius: 8px;
  border: none;
  background: transparent;
  color: var(--app-text);
  text-align: left;
  cursor: pointer;
  font-size: 0.85rem;
  transition: background-color 0.12s ease;
}
.nav-item:hover {
  background: var(--p-surface-100);
}
.app-dark .nav-item:hover {
  background: var(--p-surface-800);
}
.nav-item.active {
  background: color-mix(in srgb, var(--p-primary-500) 12%, transparent);
  color: var(--p-primary-700);
}
.app-dark .nav-item.active {
  background: color-mix(in srgb, var(--p-primary-500) 16%, transparent);
  color: var(--p-primary-400);
}
.nav-icon {
  font-size: 0.9rem;
  opacity: 0.85;
}
.nav-label {
  font-weight: 500;
}

/* Content area */
.content {
  display: flex;
  flex-direction: column;
  gap: 8px;
  padding: 16px 20px;
  overflow: auto;
}
.state-msg {
  padding: 24px;
  text-align: center;
  color: var(--app-text-muted);
}
.state-error {
  color: #dc2626;
  font-size: 0.85rem;
}

.section {
  background: var(--app-surface);
  border: 1px solid var(--app-border);
  border-radius: 12px;
  padding: 16px 18px;
}
.section-title {
  font-size: 1rem;
  font-weight: 600;
  margin: 0 0 2px;
}
.section-subtitle {
  font-size: 0.82rem;
  color: var(--app-text-muted);
  margin: 0 0 12px;
}

.field {
  display: flex;
  flex-direction: column;
  gap: 4px;
  margin-top: 10px;
}
.field-label {
  font-size: 0.78rem;
  color: var(--app-text-muted);
}
.field-hint {
  font-size: 0.72rem;
  color: var(--app-text-muted);
  opacity: 0.8;
}
.field-input {
  width: 100%;
}
.field-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
}

.mw-block {
  border-top: 1px solid var(--app-border);
  margin-top: 8px;
}
.mw-block:first-of-type {
  border-top: none;
  margin-top: 0;
}
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
.app-dark .mw-header:hover {
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
.app-dark .mw-status {
  background: var(--p-surface-800);
}
.mw-status.on {
  color: var(--p-primary-700);
  background: color-mix(in srgb, var(--p-primary-500) 14%, transparent);
}
.app-dark .mw-status.on {
  color: var(--p-primary-400);
}
.mw-toggle {
  display: flex;
  align-items: center;
}
.mw-chevron {
  margin-left: auto;
  font-size: 0.8rem;
  color: var(--app-text-muted);
}
.mw-add-btn {
  margin-left: 8px;
}
.mw-body {
  padding: 4px 4px 8px 28px;
}
.mw-empty-hint {
  font-size: 0.8rem;
  color: var(--app-text-muted);
  font-style: italic;
}

/* MCP sub-items */
.mcp-item {
  border-top: 1px dashed var(--app-border);
  margin-top: 8px;
  padding-top: 8px;
}
.mcp-item:first-of-type {
  border-top: none;
  margin-top: 0;
  padding-top: 8px;
}
.mcp-item-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  padding: 4px 0;
}
.mcp-item-label {
  font-size: 0.82rem;
  font-weight: 500;
  font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
  color: var(--app-text);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.mcp-item-actions {
  display: flex;
  align-items: center;
  gap: 6px;
}
.mcp-item-delete {
  width: 26px;
  height: 26px;
  border: none;
  border-radius: 6px;
  background: transparent;
  color: var(--app-text-muted);
  cursor: pointer;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  font-size: 0.8rem;
}
.mcp-item-delete:hover {
  background: var(--p-surface-100);
  color: #dc2626;
}
.app-dark .mcp-item-delete:hover {
  background: var(--p-surface-800);
}
.mcp-item-body {
  padding: 4px 0 8px;
}

.env-list {
  display: flex;
  flex-direction: column;
  gap: 6px;
}
.env-row,
.tool-row {
  display: flex;
  gap: 6px;
  align-items: center;
}
.env-key {
  flex: 0 0 38%;
}
.env-val {
  flex: 1;
}
.tool-info {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 1px;
}
.tool-name-display {
  font-size: 0.82rem;
  font-weight: 500;
  font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
}
.tool-desc {
  font-size: 0.72rem;
  color: var(--app-text-muted);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.env-remove {
  flex-shrink: 0;
  width: 28px;
  height: 28px;
  border: none;
  border-radius: 6px;
  background: transparent;
  color: var(--app-text-muted);
  cursor: pointer;
  display: inline-flex;
  align-items: center;
  justify-content: center;
}
.env-remove:hover {
  background: var(--p-surface-100);
  color: #dc2626;
}
.app-dark .env-remove:hover {
  background: var(--p-surface-800);
}
.env-add {
  align-self: flex-start;
  border: 1px dashed var(--app-border);
  border-radius: 8px;
  background: transparent;
  color: var(--app-text-muted);
  font-size: 0.8rem;
  padding: 6px 10px;
  cursor: pointer;
  display: inline-flex;
  align-items: center;
  gap: 6px;
  margin-top: 6px;
}
.env-add:hover {
  border-color: var(--p-primary-500);
  color: var(--p-primary-500);
}

.actions {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
  margin-top: 12px;
}
</style>
<script setup lang="ts">
import { computed } from 'vue'
import {
  activateThread, createMiddleware, deleteMiddleware, getModels, getThread,
  listMiddlewares, probeMcp, updateMiddleware, updateThread,
} from '@/api'
import type { Middleware } from '@/api/types'
import type { DynamicDialogInstance } from 'primevue/dynamicdialogoptions'

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

// Hardcoded middleware kinds. The list is a fixed Shell / Tool / MCP set; each
// item is a collapsible row with an enable toggle and a typed config form.
const MW_NAMES = ['shell', 'tool', 'mcp'] as const
type MwName = (typeof MW_NAMES)[number]

interface MwEntry {
  id: number | null // null = newly created, not yet persisted
  config: Record<string, unknown>
  original: Record<string, unknown>
  envsRows: { key: string; value: string }[]
  toolsRows: { name: string; description: string; enabled: boolean }[] // MCP only
}

const enabled = reactive<Record<MwName, boolean>>({ shell: false, tool: false, mcp: false })
const expanded = reactive<Record<MwName, boolean>>({ shell: false, tool: false, mcp: false })
const entries = reactive<Record<MwName, MwEntry | null>>({ shell: null, tool: null, mcp: null })

// MCP probe state.
const probing = ref(false)
const probeError = ref('')

const modelChanged = computed(() => model.value !== originalModel.value)
const titleChanged = computed(() => title.value !== originalTitle.value)

function splitEnvs(cfg: Record<string, unknown>): { key: string; value: string }[] {
  const envs = (cfg.envs as Record<string, string> | undefined) ?? {}
  return Object.entries(envs).map(([key, value]) => ({ key, value: String(value ?? '') }))
}
function splitTools(cfg: Record<string, unknown>): { name: string; description: string; enabled: boolean }[] {
  const tools = (cfg.toolsEnabled as Record<string, boolean> | undefined) ?? {}
  return Object.entries(tools).map(([name, on]) => ({ name, description: '', enabled: !!on }))
}
function setField(cfg: Record<string, unknown>, key: string, value: unknown): void {
  cfg[key] = value
}

function defaultConfig(name: MwName): Record<string, unknown> {
  switch (name) {
    case 'shell':
      return { workingDirectory: null, shell: null, timeoutSecs: null, envs: {} }
    case 'tool':
      return { workingDirectory: null }
    case 'mcp':
      return { transport: 'stdio', serverCommand: '', serverUrl: '', envs: {}, toolsEnabled: {} }
  }
}

function makeEntry(name: MwName, config: Record<string, unknown>, id: number | null): MwEntry {
  return {
    id,
    config: { ...config },
    original: { ...config },
    envsRows: splitEnvs(config),
    toolsRows: splitTools(config),
  }
}

const LABELS: Record<MwName, string> = { shell: 'Shell', tool: 'Tool', mcp: 'MCP' }
const ICONS: Record<MwName, string> = { shell: 'pi-terminal', tool: 'pi-wrench', mcp: 'pi-bolt' }

const TRANSPORT_OPTIONS: { label: string; value: string; icon: string }[] = [
  { label: 'Stdio', value: 'stdio', icon: 'pi-terminal' },
  { label: 'HTTP', value: 'http', icon: 'pi-globe' },
]

onMounted(async () => {
  if (threadId == null) return
  try {
    const [thread, mws, modelList] = await Promise.all([
      getThread(threadId),
      listMiddlewares(threadId),
      getModels(),
    ])
    title.value = thread.title ?? ''
    originalTitle.value = title.value
    model.value = ''
    originalModel.value = ''
    models.value = modelList

    // Group by name (take the first of each kind).
    const byName: Partial<Record<MwName, Middleware>> = {}
    for (const m of mws) {
      if ((MW_NAMES as readonly string[]).includes(m.name) && !byName[m.name as MwName]) {
        byName[m.name as MwName] = m
      }
    }
    for (const name of MW_NAMES) {
      const m = byName[name]
      if (m) {
        enabled[name] = true
        entries[name] = makeEntry(name, m.config, m.id)
      } else {
        enabled[name] = false
        entries[name] = null
      }
    }
  } catch (e) {
    error.value = e instanceof Error ? e.message : String(e)
  } finally {
    loading.value = false
  }
})

function toggleExpand(name: MwName) {
  expanded[name] = !expanded[name]
}

function onEnableToggle(name: MwName, on: boolean) {
  enabled[name] = on
  if (on) {
    if (!entries[name]) entries[name] = makeEntry(name, defaultConfig(name), null)
    expanded[name] = true
  } else {
    expanded[name] = false
  }
}

function configDiffers(entry: MwEntry): boolean {
  return JSON.stringify(entry.config) !== JSON.stringify(entry.original)
}

function toolsChanged(entry: MwEntry): boolean {
  const current: Record<string, boolean> = {}
  for (const row of entry.toolsRows) {
    if (row.name.trim()) current[row.name] = row.enabled
  }
  return JSON.stringify(current) !== JSON.stringify((entry.original.toolsEnabled as Record<string, boolean> | undefined) ?? {})
}

const middlewareChanged = computed(() => {
  for (const name of MW_NAMES) {
    if (enabled[name]) {
      const e = entries[name]
      if (!e || e.id == null || configDiffers(e) || (name === 'mcp' && toolsChanged(e))) return true
    } else if (entries[name]?.id != null) {
      return true // was persisted, now disabled → will be deleted
    }
  }
  return false
})
const dirty = computed(() => titleChanged.value || modelChanged.value || middlewareChanged.value)

function flushEnvs(entry: MwEntry, name: MwName) {
  if (name !== 'shell' && name !== 'mcp') return
  const envs: Record<string, string> = {}
  for (const row of entry.envsRows) {
    const key = row.key.trim()
    if (key) envs[key] = row.value
  }
  setField(entry.config, 'envs', envs)
}
function flushTools(entry: MwEntry, name: MwName) {
  if (name !== 'mcp') return
  const toolsEnabled: Record<string, boolean> = {}
  for (const row of entry.toolsRows) {
    const n = row.name.trim()
    if (n) toolsEnabled[n] = row.enabled
  }
  setField(entry.config, 'toolsEnabled', toolsEnabled)
}

function addEnvRow(entry: MwEntry) {
  entry.envsRows.push({ key: '', value: '' })
}
function removeEnvRow(entry: MwEntry, index: number) {
  entry.envsRows.splice(index, 1)
}

// Probe the MCP server with the current (unsaved) config and replace the tool
// list with whatever the server advertises. Preserves the enabled state of
// already-known tools; newly discovered tools default to enabled.
async function testConnection(entry: MwEntry) {
  probing.value = true
  probeError.value = ''
  flushEnvs(entry, 'mcp')
  const transport = (entry.config.transport as string) || 'stdio'
  const serverCommand = (entry.config.serverCommand as string) || null
  const serverUrl = (entry.config.serverUrl as string) || null
  const envs = (entry.config.envs as Record<string, string>) ?? {}
  try {
    const tools = await probeMcp(transport, serverCommand, serverUrl, envs)
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
    for (const name of MW_NAMES) {
      const e = entries[name]
      if (enabled[name]) {
        if (!e) continue
        flushEnvs(e, name)
        flushTools(e, name)
        if (e.id == null) {
          await createMiddleware(threadId, name, e.config)
          needsReactivation = true
        } else if (configDiffers(e)) {
          await updateMiddleware(e.id, e.config)
          needsReactivation = true
        }
      } else if (e?.id != null) {
        await deleteMiddleware(e.id)
        needsReactivation = true
      }
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
    <!-- Sidebar nav -->
    <nav class="sidebar">
      <button
        type="button"
        class="nav-item"
        :class="{ active: activeSection === 'basic' }"
        @click="activeSection = 'basic'"
      >
        <i class="pi pi-info-circle nav-icon"></i>
        <span class="nav-label">Basic Info</span>
      </button>
      <button
        type="button"
        class="nav-item"
        :class="{ active: activeSection === 'middlewares' }"
        @click="activeSection = 'middlewares'"
      >
        <i class="pi pi-sliders-h nav-icon"></i>
        <span class="nav-label">Middlewares</span>
      </button>
    </nav>

    <!-- Content area -->
    <main class="content">
      <div v-if="loading" class="state-msg">Loading…</div>
      <div v-else-if="error && !entries.shell && !entries.tool && !entries.mcp" class="state-msg state-error">
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
            <Select
              v-model="model"
              :options="models"
              option-label="undefined"
              placeholder="Select a model"
              class="field-input"
            />
          </div>
        </section>

        <!-- Middlewares section -->
        <section v-show="activeSection === 'middlewares'" class="section">
          <h2 class="section-title">Middlewares</h2>
          <p class="section-subtitle">Enable and configure per-thread middleware.</p>

          <div v-for="name in MW_NAMES" :key="name" class="mw-block">
            <div class="mw-header" @click="toggleExpand(name)">
              <i class="pi mw-icon" :class="ICONS[name]"></i>
              <span class="mw-name">{{ LABELS[name] }}</span>
              <span class="mw-status" :class="{ on: enabled[name] }">
                {{ enabled[name] ? 'Enabled' : 'Disabled' }}
              </span>
              <div class="mw-toggle" @click.stop>
                <ToggleSwitch
                  :model-value="enabled[name]"
                  @update:model-value="(v) => onEnableToggle(name, v as boolean)"
                />
              </div>
              <i class="pi mw-chevron" :class="expanded[name] ? 'pi-chevron-up' : 'pi-chevron-down'"></i>
            </div>

            <div v-show="expanded[name]" class="mw-body">
              <template v-if="enabled[name] && entries[name]">
                <!-- shell -->
                <template v-if="name === 'shell'">
                  <div class="field">
                    <label class="field-label">Working directory</label>
                    <InputText v-model="(entries[name]!.config.workingDirectory as string | undefined)" class="field-input" placeholder="(inherit server cwd)" />
                  </div>
                  <div class="field">
                    <label class="field-label">Shell</label>
                    <InputText v-model="(entries[name]!.config.shell as string | undefined)" class="field-input" placeholder="bash" />
                  </div>
                  <div class="field">
                    <label class="field-label">Timeout (seconds)</label>
                    <InputText
                      :model-value="entries[name]!.config.timeoutSecs == null ? '' : String(entries[name]!.config.timeoutSecs)"
                      @update:model-value="(v: string | undefined) => setField(entries[name]!.config, 'timeoutSecs', v === '' || v === undefined ? null : Number(v))"
                      class="field-input"
                      placeholder="(no timeout)"
                    />
                  </div>
                  <div class="field">
                    <label class="field-label">Environment variables</label>
                    <div class="env-list">
                      <div v-for="(row, i) in entries[name]!.envsRows" :key="i" class="env-row">
                        <InputText v-model="row.key" class="env-key" placeholder="KEY" />
                        <InputText v-model="row.value" class="env-val" placeholder="value" />
                        <button type="button" class="env-remove" title="Remove" @click="removeEnvRow(entries[name]!, i)">
                          <i class="pi pi-times"></i>
                        </button>
                      </div>
                      <button type="button" class="env-add" @click="addEnvRow(entries[name]!)">
                        <i class="pi pi-plus"></i> Add variable
                      </button>
                    </div>
                  </div>
                </template>

                <!-- tool -->
                <template v-else-if="name === 'tool'">
                  <div class="field">
                    <label class="field-label">Working directory</label>
                    <InputText v-model="(entries[name]!.config.workingDirectory as string | undefined)" class="field-input" placeholder="(inherit server cwd)" />
                  </div>
                </template>

                <!-- mcp -->
                <template v-else-if="name === 'mcp'">
                  <!-- Transport selector: HTTP or stdio are mutually exclusive. -->
                  <div class="field">
                    <label class="field-label">Transport</label>
                    <SelectButton
                      :model-value="(entries[name]!.config.transport as string) || 'stdio'"
                      :options="TRANSPORT_OPTIONS"
                      option-value="value"
                      option-label="label"
                      @update:model-value="(v) => setField(entries[name]!.config, 'transport', v)"
                    >
                      <template #option="{ option }">
                        <i class="pi" :class="option.icon"></i>
                        <span class="ml-1">{{ option.label }}</span>
                      </template>
                    </SelectButton>
                  </div>

                  <!-- HTTP transport -->
                  <template v-if="((entries[name]!.config.transport as string) || 'stdio') === 'http'">
                    <div class="field">
                      <label class="field-label">Server URL</label>
                      <InputText
                        v-model="(entries[name]!.config.serverUrl as string | undefined)"
                        class="field-input"
                        placeholder="http://localhost:8080/mcp"
                      />
                      <span class="field-hint">Streamable HTTP endpoint for the MCP server.</span>
                    </div>
                  </template>

                  <!-- Stdio transport -->
                  <template v-else>
                    <div class="field">
                      <label class="field-label">Server command</label>
                      <InputText
                        v-model="(entries[name]!.config.serverCommand as string | undefined)"
                        class="field-input"
                        placeholder="npx -y @modelcontextprotocol/server-filesystem"
                      />
                      <span class="field-hint">Shell command to spawn the MCP server over stdio.</span>
                    </div>
                    <div class="field">
                      <label class="field-label">Environment variables</label>
                      <div class="env-list">
                        <div v-for="(row, i) in entries[name]!.envsRows" :key="i" class="env-row">
                          <InputText v-model="row.key" class="env-key" placeholder="KEY" />
                          <InputText v-model="row.value" class="env-val" placeholder="value" />
                          <button type="button" class="env-remove" title="Remove" @click="removeEnvRow(entries[name]!, i)">
                            <i class="pi pi-times"></i>
                          </button>
                        </div>
                        <button type="button" class="env-add" @click="addEnvRow(entries[name]!)">
                          <i class="pi pi-plus"></i> Add variable
                        </button>
                      </div>
                    </div>
                  </template>

                  <div class="field">
                    <div class="field-row">
                      <label class="field-label">Tools</label>
                      <Button
                        label="Test connection"
                        icon="pi pi-bolt"
                        size="small"
                        severity="secondary"
                        :loading="probing"
                        @click="entries[name] && testConnection(entries[name]!)"
                      />
                    </div>
                    <span class="field-hint">Tools are discovered from the server — click "Test connection" to probe. Toggle which ones the model can use.</span>
                    <div v-if="probeError" class="state-error">{{ probeError }}</div>
                    <div class="env-list">
                      <div v-for="(row, i) in entries[name]!.toolsRows" :key="i" class="tool-row">
                        <div class="tool-info">
                          <span class="tool-name-display">{{ row.name }}</span>
                          <span v-if="row.description" class="tool-desc">{{ row.description }}</span>
                        </div>
                        <ToggleSwitch v-model="row.enabled" />
                      </div>
                      <div v-if="!entries[name]!.toolsRows.length" class="mw-empty-hint">
                        No tools discovered yet.
                      </div>
                    </div>
                  </div>
                </template>
              </template>
              <div v-else class="mw-empty-hint">Enable to configure.</div>
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
.nav-item:hover { background: var(--p-surface-100); }
.app-dark .nav-item:hover { background: var(--p-surface-800); }
.nav-item.active {
  background: color-mix(in srgb, var(--p-primary-500) 12%, transparent);
  color: var(--p-primary-700);
}
.app-dark .nav-item.active {
  background: color-mix(in srgb, var(--p-primary-500) 16%, transparent);
  color: var(--p-primary-400);
}
.nav-icon { font-size: 0.9rem; opacity: 0.85; }
.nav-label { font-weight: 500; }

/* Content area */
.content {
  display: flex;
  flex-direction: column;
  gap: 8px;
  padding: 16px 20px;
  overflow: auto;
}
.state-msg { padding: 24px; text-align: center; color: var(--app-text-muted); }
.state-error { color: #dc2626; font-size: 0.85rem; }

.section {
  background: var(--app-surface);
  border: 1px solid var(--app-border);
  border-radius: 12px;
  padding: 16px 18px;
}
.section-title { font-size: 1rem; font-weight: 600; margin: 0 0 2px; }
.section-subtitle { font-size: 0.82rem; color: var(--app-text-muted); margin: 0 0 12px; }

.field {
  display: flex;
  flex-direction: column;
  gap: 4px;
  margin-top: 10px;
}
.field-label { font-size: 0.78rem; color: var(--app-text-muted); }
.field-hint { font-size: 0.72rem; color: var(--app-text-muted); opacity: 0.8; }
.field-input { width: 100%; }
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
.mw-block:first-of-type { border-top: none; margin-top: 0; }
.mw-header {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 10px 4px;
  cursor: pointer;
  border-radius: 8px;
  transition: background-color 0.12s ease;
}
.mw-header:hover { background: var(--p-surface-100); }
.app-dark .mw-header:hover { background: var(--p-surface-800); }
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
.app-dark .mw-status { background: var(--p-surface-800); }
.mw-status.on {
  color: var(--p-primary-700);
  background: color-mix(in srgb, var(--p-primary-500) 14%, transparent);
}
.app-dark .mw-status.on { color: var(--p-primary-400); }
.mw-toggle { display: flex; align-items: center; }
.mw-chevron {
  margin-left: auto;
  font-size: 0.8rem;
  color: var(--app-text-muted);
}
.mw-body {
  padding: 4px 4px 8px 28px;
}
.mw-empty-hint {
  font-size: 0.8rem;
  color: var(--app-text-muted);
  font-style: italic;
}

.env-list { display: flex; flex-direction: column; gap: 6px; }
.env-row, .tool-row { display: flex; gap: 6px; align-items: center; }
.env-key { flex: 0 0 38%; }
.env-val { flex: 1; }
.tool-info { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 1px; }
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
.env-remove:hover { background: var(--p-surface-100); color: #dc2626; }
.app-dark .env-remove:hover { background: var(--p-surface-800); }
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
.raw-json {
  font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
  font-size: 0.8rem;
}

.actions {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
  margin-top: 12px;
}
.mt-3 { margin-top: 12px; }
</style>
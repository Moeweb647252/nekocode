<script setup lang="ts">
import { computed } from 'vue'
import {
  activateThread, createMiddleware, deleteMiddleware, getModels, getThread,
  listMiddlewares, updateMiddleware, updateThread,
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

interface MwDraft {
  id: number | null // null = newly created, not yet persisted
  name: string
  config: Record<string, unknown>
  original: Record<string, unknown>
  envsRows: { key: string; value: string }[]
  toolsRows: { name: string; enabled: boolean }[] // MCP only
}
const middlewares = ref<MwDraft[]>([])
const deletedIds = ref<number[]>([])

const modelChanged = computed(() => model.value !== originalModel.value)
const titleChanged = computed(() => title.value !== originalTitle.value)

function splitEnvs(cfg: Record<string, unknown>): { key: string; value: string }[] {
  const envs = (cfg.envs as Record<string, string> | undefined) ?? {}
  return Object.entries(envs).map(([key, value]) => ({ key, value: String(value ?? '') }))
}

function splitTools(cfg: Record<string, unknown>): { name: string; enabled: boolean }[] {
  const tools = (cfg.toolsEnabled as Record<string, boolean> | undefined) ?? {}
  return Object.entries(tools).map(([name, enabled]) => ({ name, enabled: !!enabled }))
}

function setField(cfg: Record<string, unknown>, key: string, value: unknown): void {
  cfg[key] = value
}

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
    middlewares.value = mws.map((m: Middleware) => ({
      id: m.id,
      name: m.name,
      config: { ...m.config },
      original: { ...m.config },
      envsRows: splitEnvs(m.config),
      toolsRows: splitTools(m.config),
    }))
  } catch (e) {
    error.value = e instanceof Error ? e.message : String(e)
  } finally {
    loading.value = false
  }
})

function configDiffers(draft: MwDraft): boolean {
  return JSON.stringify(draft.config) !== JSON.stringify(draft.original)
}

const middlewareChanged = computed(
  () =>
    middlewares.value.some((d) => configDiffers(d)) ||
    middlewares.value.some((d) => d.id == null) ||
    deletedIds.value.length > 0,
)
const dirty = computed(() => titleChanged.value || modelChanged.value || middlewareChanged.value)

// Sync env rows into the config's `envs` before save (shell + mcp both use envs).
function flushEnvs(draft: MwDraft) {
  if (draft.name !== 'shell' && draft.name !== 'mcp') return
  const envs: Record<string, string> = {}
  for (const row of draft.envsRows) {
    const key = row.key.trim()
    if (key) envs[key] = row.value
  }
  setField(draft.config, 'envs', envs)
}

// Sync tool rows into the mcp config's `toolsEnabled`.
function flushTools(draft: MwDraft) {
  if (draft.name !== 'mcp') return
  const toolsEnabled: Record<string, boolean> = {}
  for (const row of draft.toolsRows) {
    const name = row.name.trim()
    if (name) toolsEnabled[name] = row.enabled
  }
  setField(draft.config, 'toolsEnabled', toolsEnabled)
}

function addEnvRow(draft: MwDraft) {
  draft.envsRows.push({ key: '', value: '' })
}
function removeEnvRow(draft: MwDraft, index: number) {
  draft.envsRows.splice(index, 1)
}
function addToolRow(draft: MwDraft) {
  draft.toolsRows.push({ name: '', enabled: true })
}
function removeToolRow(draft: MwDraft, index: number) {
  draft.toolsRows.splice(index, 1)
}

function addMcpMiddleware() {
  middlewares.value.push({
    id: null,
    name: 'mcp',
    config: { serverCommand: '', serverUrl: '', envs: {}, toolsEnabled: {} },
    original: { serverCommand: '', serverUrl: '', envs: {}, toolsEnabled: {} },
    envsRows: [],
    toolsRows: [],
  })
}

function removeMiddleware(draft: MwDraft, index: number) {
  if (draft.id != null) deletedIds.value.push(draft.id)
  middlewares.value.splice(index, 1)
}

async function save() {
  if (threadId == null || saving.value || !dirty.value) {
    dialogRef?.value?.close(false)
    return
  }
  saving.value = true
  error.value = ''
  try {
    if (titleChanged.value) {
      await updateThread(threadId, title.value, null)
    }
    if (modelChanged.value) {
      await updateThread(threadId, null, model.value)
    }
    let needsReactivation = modelChanged.value
    for (const draft of middlewares.value) {
      flushEnvs(draft)
      flushTools(draft)
      if (draft.id == null) {
        await createMiddleware(threadId, draft.name, draft.config)
        needsReactivation = true
      } else if (configDiffers(draft)) {
        await updateMiddleware(draft.id, draft.config)
        needsReactivation = true
      }
    }
    for (const id of deletedIds.value) {
      await deleteMiddleware(id)
      needsReactivation = true
    }
    if (needsReactivation) {
      await activateThread(threadId)
    }
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
      <div v-else-if="error && !middlewares.length" class="state-msg state-error">{{ error }}</div>
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
          <p class="section-subtitle">Per-thread middleware configuration.</p>

          <div v-for="(draft, idx) in middlewares" :key="draft.id ?? `new-${idx}`" class="mw-block">
            <div class="mw-header">
              <div class="mw-name">{{ draft.name }}</div>
              <button
                type="button"
                class="mw-delete"
                title="Remove middleware"
                @click="removeMiddleware(draft, idx)"
              >
                <i class="pi pi-trash"></i>
              </button>
            </div>

            <!-- shell -->
            <template v-if="draft.name === 'shell'">
              <div class="field">
                <label class="field-label">Working directory</label>
                <InputText v-model="(draft.config.workingDirectory as string | undefined)" class="field-input" placeholder="(inherit server cwd)" />
              </div>
              <div class="field">
                <label class="field-label">Shell</label>
                <InputText v-model="(draft.config.shell as string | undefined)" class="field-input" placeholder="bash" />
              </div>
              <div class="field">
                <label class="field-label">Timeout (seconds)</label>
                <InputText
                  :model-value="draft.config.timeoutSecs == null ? '' : String(draft.config.timeoutSecs)"
                  @update:model-value="(v: string | undefined) => setField(draft.config, 'timeoutSecs', v === '' || v === undefined ? null : Number(v))"
                  class="field-input"
                  placeholder="(no timeout)"
                />
              </div>
              <div class="field">
                <label class="field-label">Environment variables</label>
                <div class="env-list">
                  <div v-for="(row, i) in draft.envsRows" :key="i" class="env-row">
                    <InputText v-model="row.key" class="env-key" placeholder="KEY" />
                    <InputText v-model="row.value" class="env-val" placeholder="value" />
                    <button type="button" class="env-remove" title="Remove" @click="removeEnvRow(draft, i)">
                      <i class="pi pi-times"></i>
                    </button>
                  </div>
                  <button type="button" class="env-add" @click="addEnvRow(draft)">
                    <i class="pi pi-plus"></i> Add variable
                  </button>
                </div>
              </div>
            </template>

            <!-- tool -->
            <template v-else-if="draft.name === 'tool'">
              <div class="field">
                <label class="field-label">Working directory</label>
                <InputText v-model="(draft.config.workingDirectory as string | undefined)" class="field-input" placeholder="(inherit server cwd)" />
              </div>
            </template>

            <!-- mcp -->
            <template v-else-if="draft.name === 'mcp'">
              <div class="field">
                <label class="field-label">Server URL (HTTP)</label>
                <InputText
                  v-model="(draft.config.serverUrl as string | undefined)"
                  class="field-input"
                  placeholder="http://localhost:8080/mcp"
                />
                <span class="field-hint">Streamable HTTP transport. Takes precedence over the command below.</span>
              </div>
              <div class="field">
                <label class="field-label">Server command (stdio)</label>
                <InputText
                  v-model="(draft.config.serverCommand as string | undefined)"
                  class="field-input"
                  placeholder="npx -y @modelcontextprotocol/server-filesystem"
                />
                <span class="field-hint">Shell command to spawn the MCP server over stdio.</span>
              </div>
              <div class="field">
                <label class="field-label">Environment variables</label>
                <div class="env-list">
                  <div v-for="(row, i) in draft.envsRows" :key="i" class="env-row">
                    <InputText v-model="row.key" class="env-key" placeholder="KEY" />
                    <InputText v-model="row.value" class="env-val" placeholder="value" />
                    <button type="button" class="env-remove" title="Remove" @click="removeEnvRow(draft, i)">
                      <i class="pi pi-times"></i>
                    </button>
                  </div>
                  <button type="button" class="env-add" @click="addEnvRow(draft)">
                    <i class="pi pi-plus"></i> Add variable
                  </button>
                </div>
              </div>
              <div class="field">
                <label class="field-label">Enabled tools</label>
                <span class="field-hint">Tools are discovered from the server at runtime. Only tools listed here with the toggle on are exposed to the model.</span>
                <div class="env-list">
                  <div v-for="(row, i) in draft.toolsRows" :key="i" class="tool-row">
                    <InputText v-model="row.name" class="tool-name" placeholder="tool_name" />
                    <ToggleSwitch v-model="row.enabled" />
                    <button type="button" class="env-remove" title="Remove" @click="removeToolRow(draft, i)">
                      <i class="pi pi-times"></i>
                    </button>
                  </div>
                  <button type="button" class="env-add" @click="addToolRow(draft)">
                    <i class="pi pi-plus"></i> Add tool
                  </button>
                </div>
              </div>
            </template>

            <!-- fallback: raw JSON -->
            <template v-else>
              <Textarea
                :model-value="JSON.stringify(draft.config, null, 2)"
                @update:model-value="(v: string) => { try { draft.config = JSON.parse(v) } catch {} }"
                rows="5"
                class="field-input raw-json"
              />
            </template>
          </div>

          <!-- Add MCP button -->
          <button type="button" class="mw-add" @click="addMcpMiddleware">
            <i class="pi pi-plus"></i> Add MCP Server
          </button>

          <div v-if="!middlewares.length && !deletedIds.length" class="section-subtitle mt-2">
            No middlewares configured.
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

.mw-block {
  border-top: 1px solid var(--app-border);
  padding-top: 12px;
  margin-top: 12px;
}
.mw-block:first-of-type { border-top: none; padding-top: 0; margin-top: 0; }
.mw-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-bottom: 4px;
}
.mw-name {
  font-size: 0.82rem;
  font-weight: 600;
  text-transform: uppercase;
  letter-spacing: 0.04em;
  color: var(--p-primary-500);
}
.mw-delete {
  flex-shrink: 0;
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
.mw-delete:hover { background: var(--p-surface-100); color: #dc2626; }
.app-dark .mw-delete:hover { background: var(--p-surface-800); }

.env-list { display: flex; flex-direction: column; gap: 6px; }
.env-row, .tool-row { display: flex; gap: 6px; align-items: center; }
.env-key { flex: 0 0 38%; }
.env-val { flex: 1; }
.tool-name { flex: 1; }
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
.env-add, .mw-add {
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
.env-add:hover, .mw-add:hover {
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
.mt-2 { margin-top: 8px; }
.mt-3 { margin-top: 12px; }
</style>
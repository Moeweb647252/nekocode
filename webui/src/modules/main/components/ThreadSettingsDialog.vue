<script setup lang="ts">
import { computed } from "vue";
import {
  activateThread,
  createMiddleware,
  deleteMiddleware,
  getModels,
  getThread,
  listMiddlewares,
  probeMcp,
  updateMiddleware,
  updateThread,
} from "@/api";
import type { DynamicDialogInstance } from "primevue/dynamicdialogoptions";

const dialogRef = inject<Ref<DynamicDialogInstance>>("dialogRef");
const threadId = (dialogRef?.value?.data as { threadId?: number } | undefined)?.threadId;

const models = ref<string[]>([]);
const loading = ref(true);
const saving = ref(false);
const error = ref("");

const title = ref("");
const originalTitle = ref("");
const model = ref("");
const originalModel = ref("");

const activeSection = ref<"basic" | "middlewares">("basic");

// Singleton middlewares: Shell and Tool are at most one per thread.
// Their enabled flag lives on the DB row; toggling updates the row.
interface SingletonEntry {
  id: number | null;
  enabled: boolean;
  originalEnabled: boolean;
  config: Record<string, unknown>;
  original: Record<string, unknown>;
  envsRows: { key: string; value: string }[];
}

const shellEntry = ref<SingletonEntry | null>(null);
const toolEntry = ref<SingletonEntry | null>(null);
const shellExpanded = ref(false);
const toolExpanded = ref(false);
const mcpExpanded = ref(true);

// MCP middlewares: zero-or-many. Each row has its own enabled toggle.
// Deleting a row removes it; adding creates a new one.
interface McpEntry {
  id: number | null;
  enabled: boolean;
  originalEnabled: boolean;
  config: Record<string, unknown>;
  original: Record<string, unknown>;
  envsRows: { key: string; value: string }[];
  toolsRows: { name: string; description: string; enabled: boolean }[];
}
const mcpEntries = ref<McpEntry[]>([]);
const deletedMcpIds = ref<number[]>([]);

const probing = ref(false);
const probeError = ref("");

const modelChanged = computed(() => model.value !== originalModel.value);
const titleChanged = computed(() => title.value !== originalTitle.value);

function splitEnvs(cfg: Record<string, unknown>): { key: string; value: string }[] {
  const envs = (cfg.envs as Record<string, string> | undefined) ?? {};
  return Object.entries(envs).map(([key, value]) => ({ key, value: String(value ?? "") }));
}
function splitTools(
  cfg: Record<string, unknown>,
): { name: string; description: string; enabled: boolean }[] {
  const tools = (cfg.toolsEnabled as Record<string, boolean> | undefined) ?? {};
  return Object.entries(tools).map(([name, on]) => ({ name, description: "", enabled: !!on }));
}
function setField(cfg: Record<string, unknown>, key: string, value: unknown): void {
  cfg[key] = value;
}

function defaultMcpConfig(): Record<string, unknown> {
  return { transport: "stdio", serverCommand: "", serverUrl: "", envs: {}, toolsEnabled: {} };
}

const TRANSPORT_OPTIONS: { label: string; value: string; icon: string }[] = [
  { label: "Stdio", value: "stdio", icon: "pi-terminal" },
  { label: "HTTP", value: "http", icon: "pi-globe" },
];

onMounted(async () => {
  if (threadId == null) return;
  try {
    const [thread, mws, modelList] = await Promise.all([
      getThread(threadId),
      listMiddlewares(threadId),
      getModels(),
    ]);
    title.value = thread.title ?? "";
    originalTitle.value = title.value;
    model.value = thread.model ?? "";
    originalModel.value = model.value;
    models.value = modelList;

    // Partition middlewares by name.
    for (const m of mws) {
      if (m.name === "shell" && !shellEntry.value) {
        shellEntry.value = {
          id: m.id,
          enabled: m.enabled,
          originalEnabled: m.enabled,
          config: { ...m.config },
          original: { ...m.config },
          envsRows: splitEnvs(m.config),
        };
      } else if (m.name === "tool" && !toolEntry.value) {
        toolEntry.value = {
          id: m.id,
          enabled: m.enabled,
          originalEnabled: m.enabled,
          config: { ...m.config },
          original: { ...m.config },
          envsRows: [],
        };
      } else if (m.name === "mcp") {
        mcpEntries.value.push({
          id: m.id,
          enabled: m.enabled,
          originalEnabled: m.enabled,
          config: { ...m.config },
          original: { ...m.config },
          envsRows: splitEnvs(m.config),
          toolsRows: splitTools(m.config),
        });
      }
    }
  } catch (e) {
    error.value = e instanceof Error ? e.message : String(e);
  } finally {
    loading.value = false;
  }
});

function singletonChanged(e: SingletonEntry): boolean {
  return JSON.stringify(e.config) !== JSON.stringify(e.original) || e.enabled !== e.originalEnabled;
}
function mcpChanged(e: McpEntry): boolean {
  return JSON.stringify(e.config) !== JSON.stringify(e.original) || e.enabled !== e.originalEnabled;
}

const middlewareChanged = computed(() => {
  if (shellEntry.value && singletonChanged(shellEntry.value)) return true;
  if (toolEntry.value && singletonChanged(toolEntry.value)) return true;
  for (const e of mcpEntries.value) {
    if (e.id == null || mcpChanged(e)) return true;
  }
  if (deletedMcpIds.value.length > 0) return true;
  return false;
});
const dirty = computed(() => titleChanged.value || modelChanged.value || middlewareChanged.value);

function flushEnvs(entry: {
  envsRows: { key: string; value: string }[];
  config: Record<string, unknown>;
}) {
  const envs: Record<string, string> = {};
  for (const row of entry.envsRows) {
    const key = row.key.trim();
    if (key) envs[key] = row.value;
  }
  setField(entry.config, "envs", envs);
}
function flushTools(entry: McpEntry) {
  const toolsEnabled: Record<string, boolean> = {};
  for (const row of entry.toolsRows) {
    const name = row.name.trim();
    if (name) toolsEnabled[name] = row.enabled;
  }
  setField(entry.config, "toolsEnabled", toolsEnabled);
}

function addEnvRow(entry: { envsRows: { key: string; value: string }[] }) {
  entry.envsRows.push({ key: "", value: "" });
}
function removeEnvRow(entry: { envsRows: { key: string; value: string }[] }, index: number) {
  entry.envsRows.splice(index, 1);
}

async function testConnection(entry: McpEntry) {
  probing.value = true;
  probeError.value = "";
  flushEnvs(entry);
  const transport = (entry.config.transport as string) || "stdio";
  const serverCommand = (entry.config.serverCommand as string) || null;
  const serverUrl = (entry.config.serverUrl as string) || null;
  const envs = (entry.config.envs as Record<string, string>) ?? {};
  try {
    const tools = await probeMcp(transport, serverCommand, serverUrl, envs);
    const prev = new Map(entry.toolsRows.map((r) => [r.name, r.enabled]));
    entry.toolsRows = tools.map((t) => ({
      name: t.name,
      description: t.description ?? "",
      enabled: prev.has(t.name) ? prev.get(t.name)! : true,
    }));
  } catch (e) {
    probeError.value = e instanceof Error ? e.message : String(e);
  } finally {
    probing.value = false;
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
    toolsRows: [],
  });
}
function removeMcpEntry(index: number) {
  const e = mcpEntries.value[index];
  if (e && e.id != null) deletedMcpIds.value.push(e.id);
  mcpEntries.value.splice(index, 1);
}

async function save() {
  if (threadId == null || saving.value || !dirty.value) {
    dialogRef?.value?.close(false);
    return;
  }
  saving.value = true;
  error.value = "";
  try {
    if (titleChanged.value) await updateThread(threadId, title.value, null);
    if (modelChanged.value) await updateThread(threadId, null, model.value);
    let needsReactivation = modelChanged.value;

    // Shell singleton.
    if (shellEntry.value) {
      const e = shellEntry.value;
      flushEnvs(e);
      if (e.id == null) {
        await createMiddleware(threadId, "shell", e.config);
        needsReactivation = true;
      } else if (singletonChanged(e)) {
        await updateMiddleware(e.id, e.config, e.enabled);
        needsReactivation = true;
      }
    }

    // Tool singleton.
    if (toolEntry.value) {
      const e = toolEntry.value;
      if (e.id == null) {
        await createMiddleware(threadId, "tool", e.config);
        needsReactivation = true;
      } else if (singletonChanged(e)) {
        await updateMiddleware(e.id, e.config, e.enabled);
        needsReactivation = true;
      }
    }

    // MCP entries: create new, update changed.
    for (const e of mcpEntries.value) {
      flushEnvs(e);
      flushTools(e);
      if (e.id == null) {
        await createMiddleware(threadId, "mcp", e.config);
        needsReactivation = true;
      } else if (mcpChanged(e)) {
        await updateMiddleware(e.id, e.config, e.enabled);
        needsReactivation = true;
      }
    }

    // MCP entries removed in the UI.
    for (const id of deletedMcpIds.value) {
      await deleteMiddleware(id);
      needsReactivation = true;
    }

    if (needsReactivation) await activateThread(threadId);
    dialogRef?.value?.close(true);
  } catch (e) {
    error.value = e instanceof Error ? e.message : String(e);
  } finally {
    saving.value = false;
  }
}

function cancel() {
  dialogRef?.value?.close(false);
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
      <div
        v-else-if="error && !shellEntry && !toolEntry && !mcpEntries.length"
        class="state-msg state-error"
      >
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
              placeholder="Select a model"
              class="field-input"
            />
          </div>
        </section>

        <!-- Middlewares section -->
        <section v-show="activeSection === 'middlewares'" class="section">
          <h2 class="section-title">Middlewares</h2>
          <p class="section-subtitle">Enable and configure per-thread middleware.</p>

          <!-- Shell (singleton) -->
          <div class="mw-block">
            <div class="mw-header" @click="shellExpanded = !shellExpanded">
              <i class="pi mw-icon pi-terminal"></i>
              <span class="mw-name">Shell</span>
              <span class="mw-status" :class="{ on: shellEntry?.enabled }">
                {{ shellEntry?.enabled ? "Enabled" : "Disabled" }}
              </span>
              <div class="mw-toggle" @click.stop>
                <ToggleSwitch
                  :model-value="shellEntry?.enabled ?? false"
                  @update:model-value="
                    (v) => {
                      if (shellEntry) shellEntry.enabled = v as boolean;
                    }
                  "
                />
              </div>
              <i
                class="pi mw-chevron"
                :class="shellExpanded ? 'pi-chevron-up' : 'pi-chevron-down'"
              ></i>
            </div>
            <div v-show="shellExpanded && shellEntry" class="mw-body">
              <div class="field">
                <label class="field-label">Working directory</label>
                <InputText
                  v-model="shellEntry!.config.workingDirectory as string | undefined"
                  class="field-input"
                  placeholder="(inherit server cwd)"
                />
              </div>
              <div class="field">
                <label class="field-label">Shell</label>
                <InputText
                  v-model="shellEntry!.config.shell as string | undefined"
                  class="field-input"
                  placeholder="bash"
                />
              </div>
              <div class="field">
                <label class="field-label">Timeout (seconds)</label>
                <InputText
                  :model-value="
                    shellEntry!.config.timeoutSecs == null
                      ? ''
                      : String(shellEntry!.config.timeoutSecs)
                  "
                  @update:model-value="
                    (v: string | undefined) =>
                      setField(
                        shellEntry!.config,
                        'timeoutSecs',
                        v === '' || v === undefined ? null : Number(v),
                      )
                  "
                  class="field-input"
                  placeholder="(no timeout)"
                />
              </div>
              <div class="field">
                <label class="field-label">Environment variables</label>
                <div class="env-list">
                  <div v-for="(row, i) in shellEntry!.envsRows" :key="i" class="env-row">
                    <InputText v-model="row.key" class="env-key" placeholder="KEY" />
                    <InputText v-model="row.value" class="env-val" placeholder="value" />
                    <button
                      type="button"
                      class="env-remove"
                      title="Remove"
                      @click="removeEnvRow(shellEntry!, i)"
                    >
                      <i class="pi pi-times"></i>
                    </button>
                  </div>
                  <button type="button" class="env-add" @click="addEnvRow(shellEntry!)">
                    <i class="pi pi-plus"></i> Add variable
                  </button>
                </div>
              </div>
            </div>
          </div>

          <!-- Tool (singleton) -->
          <div class="mw-block">
            <div class="mw-header" @click="toolExpanded = !toolExpanded">
              <i class="pi mw-icon pi-wrench"></i>
              <span class="mw-name">Tool</span>
              <span class="mw-status" :class="{ on: toolEntry?.enabled }">
                {{ toolEntry?.enabled ? "Enabled" : "Disabled" }}
              </span>
              <div class="mw-toggle" @click.stop>
                <ToggleSwitch
                  :model-value="toolEntry?.enabled ?? false"
                  @update:model-value="
                    (v) => {
                      if (toolEntry) toolEntry.enabled = v as boolean;
                    }
                  "
                />
              </div>
              <i
                class="pi mw-chevron"
                :class="toolExpanded ? 'pi-chevron-up' : 'pi-chevron-down'"
              ></i>
            </div>
            <div v-show="toolExpanded && toolEntry" class="mw-body">
              <div class="field">
                <label class="field-label">Working directory</label>
                <InputText
                  v-model="toolEntry!.config.workingDirectory as string | undefined"
                  class="field-input"
                  placeholder="(inherit server cwd)"
                />
              </div>
            </div>
          </div>

          <!-- MCP (0..n) -->
          <div class="mw-block">
            <div class="mw-header" @click="mcpExpanded = !mcpExpanded">
              <i class="pi mw-icon pi-bolt"></i>
              <span class="mw-name">MCP Servers</span>
              <Button
                label="Add"
                icon="pi pi-plus"
                size="small"
                severity="secondary"
                class="mw-add-btn"
                @click.stop="addMcpEntry"
              />
              <i
                class="pi mw-chevron"
                :class="mcpExpanded ? 'pi-chevron-up' : 'pi-chevron-down'"
              ></i>
            </div>
            <div v-show="mcpExpanded" class="mw-body">
              <div v-if="!mcpEntries.length" class="mw-empty-hint">No MCP servers configured.</div>
              <div
                v-for="(entry, idx) in mcpEntries"
                :key="entry.id ?? `new-${idx}`"
                class="mcp-item"
              >
                <div class="mcp-item-header">
                  <span class="mcp-item-label">{{
                    entry.config.serverCommand || entry.config.serverUrl || "MCP Server"
                  }}</span>
                  <div class="mcp-item-actions">
                    <ToggleSwitch v-model="entry.enabled" />
                    <button
                      type="button"
                      class="mcp-item-delete"
                      title="Remove"
                      @click="removeMcpEntry(idx)"
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
                      :model-value="(entry.config.transport as string) || 'stdio'"
                      :options="TRANSPORT_OPTIONS"
                      option-value="value"
                      option-label="label"
                      @update:model-value="(v) => setField(entry.config, 'transport', v)"
                    >
                      <template #option="{ option }">
                        <i class="pi" :class="option.icon"></i>
                        <span class="ml-1">{{ option.label }}</span>
                      </template>
                    </SelectButton>
                  </div>

                  <!-- HTTP transport -->
                  <template v-if="((entry.config.transport as string) || 'stdio') === 'http'">
                    <div class="field">
                      <label class="field-label">Server URL</label>
                      <InputText
                        v-model="entry.config.serverUrl as string | undefined"
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
                        v-model="entry.config.serverCommand as string | undefined"
                        class="field-input"
                        placeholder="npx -y @modelcontextprotocol/server-filesystem"
                      />
                      <span class="field-hint"
                        >Shell command to spawn the MCP server over stdio.</span
                      >
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
                        @click="testConnection(entry)"
                      />
                    </div>
                    <span class="field-hint"
                      >Tools are discovered from the server — click "Test connection" to probe.
                      Toggle which ones the model can use.</span
                    >
                    <div v-if="probeError" class="state-error">{{ probeError }}</div>
                    <div class="env-list">
                      <div v-for="(row, i) in entry.toolsRows" :key="i" class="tool-row">
                        <div class="tool-info">
                          <span class="tool-name-display">{{ row.name }}</span>
                          <span v-if="row.description" class="tool-desc">{{
                            row.description
                          }}</span>
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
            </div>
          </div>
        </section>

        <div v-if="error" class="state-error mt-3">{{ error }}</div>

        <div class="actions">
          <Button
            label="Cancel"
            severity="secondary"
            variant="text"
            :disabled="saving"
            @click="cancel"
          />
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
.mt-3 {
  margin-top: 12px;
}
</style>

<script setup lang="ts">
const props = defineProps<{ disabled?: boolean }>()
const input = defineModel('value', {
  type: String,
  required: true,
})
const emit = defineEmits(['sendClicked', 'settingsClicked', 'cancelClicked'])

const ta = ref<HTMLTextAreaElement | null>(null)

const onKeydown = (e: KeyboardEvent) => {
  // Enter sends; Shift+Enter inserts a newline.
  if (e.key === 'Enter' && !e.shiftKey && !e.isComposing) {
    e.preventDefault()
    if (props.disabled) return
    emit('sendClicked')
  }
}

// Auto-grow the textarea up to a max height, then scroll.
function autosize() {
  const el = ta.value
  if (!el) return
  el.style.height = 'auto'
  el.style.height = Math.min(el.scrollHeight, 240) + 'px'
}

watch(input, () => nextTick(autosize))
onMounted(() => nextTick(autosize))

const canSend = computed(() => !props.disabled && (input.value ?? '').trim().length > 0)
</script>

<template>
  <div class="px-4 pb-3 pt-2">
    <div class="mx-auto max-w-4xl">
      <div class="input-row">
        <span class="prompt-glyph" aria-hidden="true">❯</span>
        <textarea
          ref="ta"
          v-model="input"
          rows="1"
          class="input-field"
          placeholder="Describe a task, paste an error, or ask about this codebase…"
          :disabled="disabled"
          @keydown="onKeydown"
        ></textarea>
        <button
          v-if="disabled"
          type="button"
          class="send-btn cancel-btn"
          title="Stop generation"
          aria-label="Stop generation"
          @click="emit('cancelClicked')"
        >
          <i class="pi pi-stop"></i>
        </button>
        <button
          v-else
          type="button"
          class="icon-btn"
          title="Thread settings"
          aria-label="Thread settings"
          :disabled="disabled"
          @click="emit('settingsClicked')"
        >
          <i class="pi pi-cog"></i>
        </button>
        <button
          type="button"
          class="send-btn"
          :disabled="!canSend"
          title="Send (Enter)"
          aria-label="Send"
          @click="emit('sendClicked')"
        >
          <i class="pi pi-arrow-up"></i>
        </button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.input-row {
  display: flex;
  align-items: flex-end;
  gap: 8px;
  padding: 6px 6px 6px 12px;
  border-radius: 8px;
  border: 1px solid var(--app-border);
  background: var(--app-surface);
  transition:
    border-color 0.15s ease,
    box-shadow 0.15s ease;
}
.input-row:focus-within {
  border-color: var(--p-primary-500);
  box-shadow: 0 0 0 2px color-mix(in srgb, var(--p-primary-500) 18%, transparent);
}

.prompt-glyph {
  flex-shrink: 0;
  color: var(--p-primary-500);
  font-family: ui-monospace, SFMono-Regular, 'SF Mono', Menlo, Consolas, monospace;
  font-size: 0.9rem;
  line-height: 1.5;
  padding-bottom: 6px;
  padding-top: 7px;
}

.input-field {
  flex: 1;
  min-width: 0;
  resize: none;
  border: none;
  outline: none;
  background: transparent;
  color: var(--app-text);
  font-family: ui-monospace, SFMono-Regular, 'SF Mono', Menlo, Consolas, monospace;
  font-size: 0.85rem;
  line-height: 1.55;
  max-height: 240px;
  padding: 7px 0;
}
.input-field::placeholder {
  color: var(--app-text-muted);
  font-family: inherit;
}
.input-field:disabled {
  opacity: 0.6;
}

.send-btn {
  flex-shrink: 0;
  width: 30px;
  height: 30px;
  border-radius: 6px;
  border: none;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  cursor: pointer;
  color: white;
  background: var(--p-primary-500);
  transition:
    background-color 0.15s ease,
    opacity 0.15s ease;
  font-size: 0.8rem;
}
.send-btn:hover:not(:disabled) {
  background: var(--p-primary-600);
}
.send-btn:disabled {
  background: var(--p-surface-300);
  cursor: not-allowed;
}
.app-dark .send-btn:disabled {
  background: var(--p-surface-700);
}

.icon-btn {
  flex-shrink: 0;
  width: 30px;
  height: 30px;
  border-radius: 6px;
  border: none;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  cursor: pointer;
  color: var(--app-text-muted);
  background: transparent;
  transition:
    background-color 0.15s ease,
    color 0.15s ease;
  font-size: 0.85rem;
}
.icon-btn:hover:not(:disabled) {
  background: var(--p-surface-100);
  color: var(--p-primary-500);
}
.app-dark .icon-btn:hover:not(:disabled) {
  background: var(--p-surface-800);
}
.icon-btn:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}
</style>

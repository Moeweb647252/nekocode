<script setup lang="ts">
const props = defineProps<{ disabled?: boolean }>();
const input = defineModel("value", {
  type: String,
  required: true,
});
const emit = defineEmits(["sendClicked"]);

const onKeydown = (e: KeyboardEvent) => {
  // Enter sends; Shift+Enter inserts a newline.
  if (e.key === "Enter" && !e.shiftKey && !e.isComposing) {
    e.preventDefault();
    if (props.disabled) return;
    emit("sendClicked");
  }
};
</script>
<template>
  <div class="grid grid-rows-[1fr_auto] w-full h-full p-2">
    <textarea
      v-model="input"
      class="resize-none border-none focus:outline-none"
      placeholder="Send message to agent..."
      :disabled="disabled"
      @keydown="onKeydown"
    ></textarea>
    <div class="flex justify-end mt-2">
      <Button label="Send" :disabled="disabled" @click="emit('sendClicked')" />
    </div>
  </div>
</template>

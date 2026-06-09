<script setup lang="ts">
const input = defineModel("value", {
  type: String,
  required: true,
});
const emit = defineEmits(["sendClicked"]);
</script>
<template>
  <div class="grid grid-rows-[1fr_auto] w-full h-full p-2">
    <textarea
      fluid
      v-model="input"
      class="resize-none border-none focus:outline-none"
      placeholder="Send message to agent..."
    ></textarea>
    <div class="flex justify-end mt-2">
      <Button label="Send" @click="emit('sendClicked')" />
    </div>
  </div>
</template>

import { defineStore } from "pinia";

export const useAppStore = defineStore('app', {
  state: () => {
    return {
      token: undefined as string | undefined,
    }
  },
})

import { defineStore } from 'pinia'

export type ThemeMode = 'light' | 'dark' | 'system'

const THEME_KEY = 'nekocode.themeMode'
const DARK_CLASS = 'app-dark'

function readStoredMode(): ThemeMode {
  const v = typeof localStorage !== 'undefined' ? localStorage.getItem(THEME_KEY) : null
  return v === 'light' || v === 'dark' || v === 'system' ? v : 'system'
}

function systemPrefersDark(): boolean {
  return (
    typeof window !== 'undefined' &&
    typeof window.matchMedia === 'function' &&
    window.matchMedia('(prefers-color-scheme: dark)').matches
  )
}

/** Resolve the effective (concrete) mode for a stored mode. */
export function effectiveMode(mode: ThemeMode): 'light' | 'dark' {
  return mode === 'system' ? (systemPrefersDark() ? 'dark' : 'light') : mode
}

let mediaListener: ((e: MediaQueryListEvent) => void) | null = null

export const useAppStore = defineStore('app', {
  state: () => {
    return {
      token: undefined as string | undefined,
      themeMode: readStoredMode() as ThemeMode,
    }
  },
  actions: {
    /** Apply the current `themeMode` to <html> and persist it. Call on boot. */
    initTheme() {
      this.applyTheme()
      // React to OS theme changes only while in `system` mode.
      if (typeof window !== 'undefined' && typeof window.matchMedia === 'function') {
        const mql = window.matchMedia('(prefers-color-scheme: dark)')
        if (mediaListener) mql.removeEventListener('change', mediaListener)
        mediaListener = () => {
          if (this.themeMode === 'system') this.applyTheme()
        }
        mql.addEventListener('change', mediaListener)
      }
    },
    setThemeMode(mode: ThemeMode) {
      this.themeMode = mode
      try {
        localStorage.setItem(THEME_KEY, mode)
      } catch {
        /* storage may be unavailable (private mode); ignore */
      }
      this.applyTheme()
    },
    /** Toggle `.app-dark` on <html> + set `color-scheme` for native controls. */
    applyTheme() {
      if (typeof document === 'undefined') return
      const dark = effectiveMode(this.themeMode) === 'dark'
      document.documentElement.classList.toggle(DARK_CLASS, dark)
      document.documentElement.style.colorScheme = dark ? 'dark' : 'light'
    },
  },
})

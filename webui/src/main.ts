import { createApp } from 'vue'
import { createPinia } from 'pinia'
import PrimeVue from 'primevue/config'
import Aura from '@primeuix/themes/aura'
import DialogService from 'primevue/dialogservice'
import { definePreset, palette } from '@primeuix/themes'

import App from './App.vue'
import router from './router'
import { useAppStore } from './stores/app'

import 'virtual:uno.css'
import './style.scss'

// ── Theme preset ────────────────────────────────────────────────────────────
// Teal/emerald primary on a slate surface scale, layered onto the Aura preset.
// The surface palette makes light/dark surfaces token-driven so components can
// reference `--p-surface-*` instead of hard-coded grays.
const Neko = definePreset(Aura, {
  semantic: {
    primary: palette('#0d9488'), // teal-600 — fresh, "Neko" brand-ish
    colorScheme: {
      light: {
        primary: palette('#0d9488'),
        // Full slate scale (NOT palette('#f8fafc')) — palette() treats its
        // input as the 500 mid-stop, so seeding it with a near-white would
        // make surface-900 (used for text) come out too pale. Explicit stops
        // keep text dark and surfaces airy in light mode.
        surface: {
          0: '#ffffff',
          50: '#f8fafc',
          100: '#f1f5f9',
          200: '#e2e8f0',
          300: '#cbd5e1',
          400: '#94a3b8',
          500: '#64748b',
          600: '#475569',
          700: '#334155',
          800: '#1e293b',
          900: '#0f172a',
          950: '#0b1120',
        },
        formField: {
          background: '{surface.0}',
          borderColor: '{surface.200}',
        },
      },
      dark: {
        primary: palette('#14b8a6'), // teal-500 — a touch brighter on dark
        // No surface override here: Aura's default dark surface ramp already
        // steps correctly for PrimeVue components. The app's custom surfaces
        // (bubbles, cards) use explicit --app-* CSS vars (see style.scss) so
        // they don't depend on the surface ramp direction.
        formField: {
          background: '{surface.950}',
          borderColor: '{surface.800}',
        },
      },
    },
  },
})

const app = createApp(App)

app.use(createPinia())
app.use(router)
app.use(PrimeVue, {
  theme: {
    preset: Neko,
    // Toggle dark mode by toggling `.app-dark` on <html> (see stores/app.ts).
    options: {
      darkModeSelector: '.app-dark',
    },
  },
  options: {
    ripple: false,
  },
})
app.use(DialogService)

// Apply persisted/system theme before mount to avoid a flash.
useAppStore().initTheme()

app.mount('#app')

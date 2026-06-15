import {
    defineConfig,
    presetAttributify,
    presetIcons,
    transformerVariantGroup,
    transformerDirectives,
    presetWind4,
} from 'unocss'

export default defineConfig({
    presets: [
        presetAttributify(),
        presetIcons({
            warn: true,
        }),
        presetWind4(),
    ],
    transformers: [
        transformerVariantGroup(),
        transformerDirectives(),
    ],
    theme: {
        // Semantic color shortcuts so templates can write `text-brand-600`
        // / `bg-surface` instead of hard-coded grays. These mirror the teal
        // primary + slate surface scale defined in the PrimeVue preset.
        colors: {
            brand: {
                50: '#f0fdfa',
                100: '#ccfbf1',
                200: '#99f6e4',
                300: '#5eead4',
                400: '#2dd4bf',
                500: '#14b8a6',
                600: '#0d9488',
                700: '#0f766e',
                800: '#115e59',
                900: '#134e4a',
                950: '#042f2e',
            },
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
        },
    },
})

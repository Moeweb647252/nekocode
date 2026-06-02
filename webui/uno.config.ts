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
})

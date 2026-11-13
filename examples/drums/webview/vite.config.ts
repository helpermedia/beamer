import { defaultClientConditions, defineConfig } from 'vite'
import { svelte } from '@sveltejs/vite-plugin-svelte'
import tailwindcss from '@tailwindcss/vite'

export default defineConfig({
  plugins: [svelte(), tailwindcss()],
  base: './',
  // Read the Beamer packages of this repository from their TypeScript and
  // Svelte sources, so that they need no build of their own.
  resolve: {
    conditions: ['source', ...defaultClientConditions],
  },
  build: {
    outDir: 'dist',
  },
})

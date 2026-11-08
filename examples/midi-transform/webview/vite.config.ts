import { defaultClientConditions, defineConfig } from 'vite'
import react from '@vitejs/plugin-react'
import tailwindcss from '@tailwindcss/vite'

export default defineConfig({
  plugins: [react(), tailwindcss()],
  base: './',
  // Read the Beamer packages of this repository from their TypeScript
  // sources, so that they need no build of their own.
  resolve: {
    conditions: ['source', ...defaultClientConditions],
  },
  build: {
    outDir: 'dist',
  },
})

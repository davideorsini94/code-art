import { defineConfig } from 'vitest/config'
import { svelte } from '@sveltejs/vite-plugin-svelte'
import { svelteTesting } from '@testing-library/svelte/vite'

export default defineConfig({
  plugins: [svelte(), svelteTesting()],
  clearScreen: false,
  server: { port: 1420, strictPort: true },
  test: { environment: 'node' },
})

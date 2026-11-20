// Dev-only: renders LyricsStage with sample lyrics so layouts can be
// screenshotted at many window sizes (see preview/shoot.mjs).
import { defineConfig } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';
import { fileURLToPath } from 'node:url';

export default defineConfig({
  root: fileURLToPath(new URL('.', import.meta.url)),
  base: './',
  plugins: [svelte({ configFile: fileURLToPath(new URL('../svelte.config.js', import.meta.url)) })],
  build: { outDir: '../preview-dist', emptyOutDir: true, target: ['es2021', 'chrome105', 'safari14'] },
});

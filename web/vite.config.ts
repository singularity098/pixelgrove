import { defineConfig } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';

export default defineConfig({
  plugins: [svelte()],
  server: {
    // During `npm run dev`, forward API calls to a locally running `cargo run`.
    proxy: {
      '/api': 'http://localhost:8080',
    },
  },
});

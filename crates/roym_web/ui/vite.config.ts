import { defineConfig } from 'vite';

export default defineConfig({
  base: './',
  server: {
    fs: {
      allow: [
        '.',
        '../../roym_core/taxonomy',
        '../../roym_core/extensions',
      ],
    },
  },
  build: {
    outDir: 'dist',
    emptyOutDir: true,
  },
  test: {
    environment: 'jsdom',
  },
});

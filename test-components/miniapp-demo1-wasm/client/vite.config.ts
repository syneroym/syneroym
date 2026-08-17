import { defineConfig } from 'vite';
import solidPlugin from 'vite-plugin-solid';

export default defineConfig({
  plugins: [solidPlugin()],
  base: '/dist/',
  build: {
    target: 'esnext',
    outDir: '../static/dist',
    emptyOutDir: true,
  },
});

import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'

export default defineConfig({
  plugins: [react()],
  server: {
    proxy: {
      '/admin': 'http://34.63.238.145:8080',
      '/execute': 'http://34.63.238.145:8080',
      '/healthz': 'http://34.63.238.145:8080',
    },
  },
})

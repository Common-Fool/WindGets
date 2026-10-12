import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

export default defineConfig({
  plugins: [react()],
  // Tauri сам пишет в консоль, чистка экрана Vite мешает читать его вывод
  clearScreen: false,
  server: {
    // Порт фиксированный: он же прописан в tauri.conf.json как devUrl
    host: "127.0.0.1",
    port: 1420,
    strictPort: true,
    // Пересборка фронта не должна дёргаться на изменения в Rust
    watch: { ignored: ["**/src-tauri/**"] },
  },
  build: {
    // WebView2 на этой машине свежий, но занижать цель безопаснее
    target: "chrome110",
    sourcemap: false,
  },
});

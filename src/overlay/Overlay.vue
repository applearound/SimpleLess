<script setup lang="ts">
import { ref } from "vue";
import { listen } from "@tauri-apps/api/event";

type Phase = "listening" | "processing" | "result" | "error" | "hidden";
interface State {
  phase: Phase;
  text?: string;
  mode?: "dictate" | "command";
}

const state = ref<State>({ phase: "hidden" });

listen<State>("overlay://state", (e) => {
  state.value = e.payload;
});

const modeLabel = (mode?: string) => (mode === "command" ? "命令" : "听写");
</script>

<template>
  <div class="screen">
    <div v-if="state.phase !== 'hidden'" class="capsule" :class="state.phase">
      <span v-if="state.phase === 'listening'" class="dot pulse"></span>
      <span v-else-if="state.phase === 'processing'" class="dot spin"></span>
      <span v-else-if="state.phase === 'result'" class="dot ok">✓</span>
      <span v-else-if="state.phase === 'error'" class="dot err">!</span>

      <span class="mode">{{ modeLabel(state.mode) }}</span>

      <span v-if="state.phase === 'listening' && !state.text" class="text muted"
        >正在聆听，再按一次热键结束…</span
      >
      <span v-else-if="state.phase === 'processing'" class="text muted processing-text">
        {{ state.text || "正在处理…" }}
      </span>
      <span v-else-if="state.phase === 'error'" class="text err-text">{{ state.text }}</span>
      <span v-else class="text">{{ state.text }}</span>
    </div>
  </div>
</template>

<style>
html,
body,
#overlay {
  margin: 0;
  padding: 0;
  background: transparent;
  overflow: hidden;
}

.screen {
  width: 100vw;
  height: 100vh;
  display: flex;
  align-items: flex-end;
  justify-content: center;
  padding-bottom: 36px;
  box-sizing: border-box;
  pointer-events: none;
  font-family: system-ui, -apple-system, "Segoe UI", "Microsoft YaHei", sans-serif;
}

.capsule {
  display: flex;
  align-items: center;
  gap: 10px;
  max-width: 92%;
  padding: 12px 20px;
  border-radius: 14px;
  background: rgba(20, 20, 24, 0.88);
  color: #f5f5f5;
  box-shadow: 0 8px 32px rgba(0, 0, 0, 0.35);
  backdrop-filter: blur(8px);
  font-size: 15px;
}

.dot {
  width: 10px;
  height: 10px;
  border-radius: 50%;
  flex-shrink: 0;
}

.dot.pulse {
  background: #ef4444;
  animation: pulse 1.2s ease-in-out infinite;
}

.dot.spin {
  border: 2px solid rgba(96, 165, 250, 0.3);
  border-top-color: #60a5fa;
  background: transparent;
  animation: spin 0.8s linear infinite;
}

.dot.ok {
  width: auto;
  height: auto;
  background: transparent;
  color: #4ade80;
  font-weight: 700;
}

.dot.err {
  width: auto;
  height: auto;
  background: transparent;
  color: #f87171;
  font-weight: 700;
}

.mode {
  flex-shrink: 0;
  padding: 2px 8px;
  border-radius: 6px;
  background: rgba(255, 255, 255, 0.12);
  font-size: 12px;
  opacity: 0.9;
}

.text {
  white-space: pre-wrap;
  word-break: break-all;
  max-height: 72px;
  overflow: hidden;
}

.muted {
  opacity: 0.6;
}

.err-text {
  color: #f87171;
}

.processing-text::after {
  content: "▌";
  animation: blink 1s step-end infinite;
}

@keyframes pulse {
  0%,
  100% {
    opacity: 1;
    transform: scale(1);
  }
  50% {
    opacity: 0.4;
    transform: scale(0.8);
  }
}

@keyframes spin {
  to {
    transform: rotate(360deg);
  }
}

@keyframes blink {
  50% {
    opacity: 0;
  }
}
</style>

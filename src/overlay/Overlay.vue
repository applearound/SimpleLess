<script setup lang="ts">
import { ref, watch, nextTick } from "vue";
import { listen } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";
import { X } from "lucide-vue-next";

type Phase = "listening" | "finalizing" | "processing" | "result" | "error" | "hidden";
interface State {
  phase: Phase;
  text?: string;
  mode?: "dictate" | "command";
}

const state = ref<State>({ phase: "hidden" });

listen<State>("overlay://state", (e) => {
  // 收尾与润色阶段忽略迟到的监听事件，防止状态闪回红灯
  if (
    (state.value.phase === "processing" || state.value.phase === "finalizing") &&
    e.payload.phase === "listening"
  )
    return;
  state.value = e.payload;
});

// 录音剩余秒数，null 表示不在倒计时中
const countdown = ref<number | null>(null);
listen<number | null>("overlay://countdown", (e) => {
  countdown.value = e.payload;
});

const capsule = ref<HTMLElement | null>(null);
let lastHeight = 0;
// 听写起始时由占位符实测的宽度下限，避免首字出现时气泡骤缩
const minWidth = ref(0);

// 内容高度上报给 Rust 侧扩窗：36px 是 .screen 的底部留白，4px 余量
async function reportHeight() {
  const el = capsule.value;
  if (!el) return;
  const h = Math.ceil(el.getBoundingClientRect().height) + 40;
  if (Math.abs(h - lastHeight) < 2) return;
  lastHeight = h;
  try {
    await invoke("resize_overlay", { height: h });
  } catch {
    // 窗口已关闭时静默
  }
}

watch(state, async (s) => {
  await nextTick();
  if (s.phase === "hidden") {
    minWidth.value = 0;
  } else if (s.phase === "listening" && !s.text && capsule.value) {
    minWidth.value = Math.ceil(capsule.value.getBoundingClientRect().width);
  }
  reportHeight();
});

// 提前放弃润色：后端原子置位取消令牌，原文直接粘贴，迟到的润色结果被丢弃
async function cancelPolish() {
  try {
    await invoke("cancel_polish");
  } catch {
    // 会话已结束则忽略
  }
}

// 阶段标签：录音中显示模式名，收尾显示识别，润色阶段显示润色，
// 输出完成显示完成，让气泡状态与真实处理阶段始终一致
const phaseLabel = (mode?: string, phase?: Phase) => {
  if (phase === "finalizing") return "识别";
  if (phase === "processing") return mode === "command" ? "处理" : "润色";
  if (phase === "result") return "完成";
  return mode === "command" ? "命令" : "听写";
};
</script>

<template>
  <div class="screen">
    <div
      v-if="state.phase !== 'hidden'"
      ref="capsule"
      class="capsule"
      :class="state.phase"
      :style="minWidth ? { minWidth: minWidth + 'px' } : undefined"
    >
      <span v-if="state.phase === 'listening'" class="dot pulse"></span>
      <span
        v-else-if="state.phase === 'finalizing' || state.phase === 'processing'"
        class="dot pulse blue"
      ></span>
      <span v-else-if="state.phase === 'result'" class="dot ok">✓</span>
      <span v-else-if="state.phase === 'error'" class="dot err">!</span>

      <span class="mode">{{ phaseLabel(state.mode, state.phase) }}</span>
      <span
        v-if="
          countdown !== null &&
            (state.phase === 'listening' ||
              state.phase === 'finalizing' ||
              state.phase === 'processing')
        "
        class="countdown"
      >
        剩 {{ countdown }} 秒
      </span>
      <button
        v-if="state.phase === 'processing' && state.mode !== 'command'"
        class="cancel-btn"
        title="取消润色，直接使用原文"
        @click="cancelPolish"
      >
        <X class="size-3.5" />
      </button>

      <span v-if="state.phase === 'error'" class="text err-text">{{ state.text }}</span>
      <span
        v-else-if="state.phase === 'listening' && !state.text"
        class="text muted"
        >正在聆听，再按一次热键结束…</span
      >
      <span v-else-if="state.phase === 'finalizing' && !state.text" class="text muted"
        >正在完成识别…</span
      >
      <span v-else-if="state.text" class="text">{{ state.text }}</span>
      <span v-else class="text muted">正在处理…</span>
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
  box-sizing: border-box;
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

.dot.blue {
  background: #60a5fa;
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

.countdown {
  flex-shrink: 0;
  padding: 2px 8px;
  border-radius: 6px;
  background: rgba(255, 255, 255, 0.12);
  font-size: 12px;
  opacity: 0.9;
  font-variant-numeric: tabular-nums;
}

.cancel-btn {
  pointer-events: auto;
  flex-shrink: 0;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  cursor: pointer;
  border: none;
  background: transparent;
  color: rgba(245, 245, 245, 0.6);
  width: 22px;
  height: 22px;
  padding: 0;
  border-radius: 6px;
}

.cancel-btn:hover {
  background: rgba(255, 255, 255, 0.12);
  color: #f5f5f5;
}

.text {
  white-space: pre-wrap;
  word-break: break-all;
}

.muted {
  opacity: 0.6;
}

.err-text {
  color: #f87171;
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
</style>

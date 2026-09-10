<script setup lang="ts">
import { onMounted, ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";

const onboarded = ref(false);
const showKeyForm = ref(false);
const apiKey = ref("");
const saving = ref(false);
const message = ref("");
const error = ref("");
const config = ref<Record<string, unknown> | null>(null);

onMounted(async () => {
  try {
    const status = await invoke<{ onboarded: boolean; config: Record<string, unknown> }>(
      "get_setup_status",
    );
    onboarded.value = status.onboarded;
    config.value = status.config;
  } catch (e) {
    error.value = String(e);
  }
});

async function save() {
  saving.value = true;
  error.value = "";
  message.value = "";
  try {
    await invoke("save_api_key", { key: apiKey.value });
    onboarded.value = true;
    showKeyForm.value = false;
    apiKey.value = "";
    message.value = "验证通过，配置已保存。现在可以用热键开始语音输入了。";
    setTimeout(async () => {
      await getCurrentWindow().hide();
    }, 2000);
  } catch (e) {
    error.value = String(e);
  } finally {
    saving.value = false;
  }
}
</script>

<template>
  <main class="page">
    <h1>SimpleLess</h1>
    <p class="tagline">语音优先的输入工具 · 无界面操作</p>

    <section v-if="!onboarded || showKeyForm" class="card">
      <h2>{{ onboarded ? "更换 API Key" : "初次使用" }}</h2>
      <p v-if="!onboarded">
        填入阿里云百炼平台的 API Key，这是唯一一次需要键盘的配置。验证通过后，所有操作都通过语音完成。
      </p>
      <input
        v-model="apiKey"
        type="password"
        placeholder="sk-..."
        spellcheck="false"
        @keyup.enter="save"
      />
      <button :disabled="saving || !apiKey.trim()" @click="save">
        {{ saving ? "正在验证..." : "验证并保存" }}
      </button>
      <button v-if="onboarded" class="secondary" @click="showKeyForm = false">取消</button>
      <p v-if="error" class="error">{{ error }}</p>
    </section>

    <section v-else class="card">
      <h2>已就绪</h2>
      <p v-if="message" class="success">{{ message }}</p>
      <dl v-if="config" class="summary">
        <div>
          <dt>听写热键</dt>
          <dd>{{ config.hotkeyDictate }}</dd>
        </div>
        <div>
          <dt>命令热键</dt>
          <dd>{{ config.hotkeyCommand }}</dd>
        </div>
        <div>
          <dt>润色档位</dt>
          <dd>{{ config.polishMode === "polished" ? "润色" : "原文" }}</dd>
        </div>
        <div>
          <dt>识别模型</dt>
          <dd>{{ config.asrModel }}</dd>
        </div>
        <div>
          <dt>润色模型</dt>
          <dd>{{ config.llmModel }}</dd>
        </div>
      </dl>
      <p class="hint">
        按听写热键开始、再按一次结束，文本会插入当前光标处；按命令热键后用语音修改设置，比如说“切换成原文”。
      </p>
      <button class="secondary" @click="showKeyForm = true">更换 API Key</button>
    </section>
  </main>
</template>

<style scoped>
.page {
  min-height: 100vh;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 12px;
  padding: 24px;
}

h1 {
  margin: 0;
  font-size: 28px;
  letter-spacing: 0.02em;
}

.tagline {
  margin: 0 0 8px;
  opacity: 0.6;
  font-size: 13px;
}

.card {
  width: 100%;
  max-width: 420px;
  display: flex;
  flex-direction: column;
  gap: 12px;
  padding: 20px;
  border-radius: 12px;
  border: 1px solid rgba(128, 128, 128, 0.25);
}

h2 {
  margin: 0;
  font-size: 16px;
}

p {
  margin: 0;
  font-size: 13px;
  line-height: 1.6;
  opacity: 0.85;
}

input {
  padding: 10px 12px;
  border-radius: 8px;
  border: 1px solid rgba(128, 128, 128, 0.4);
  font-size: 14px;
  font-family: inherit;
}

button {
  padding: 10px 12px;
  border-radius: 8px;
  border: none;
  background: #2563eb;
  color: #fff;
  font-size: 14px;
  cursor: pointer;
}

button:disabled {
  opacity: 0.5;
  cursor: default;
}

button.secondary {
  background: transparent;
  color: inherit;
  border: 1px solid rgba(128, 128, 128, 0.4);
}

.error {
  color: #dc2626;
}

.success {
  color: #16a34a;
}

.summary {
  margin: 0;
  display: flex;
  flex-direction: column;
  gap: 6px;
  font-size: 13px;
}

.summary div {
  display: flex;
  justify-content: space-between;
}

.summary dt {
  opacity: 0.6;
}

.summary dd {
  margin: 0;
  font-family: ui-monospace, monospace;
}

.hint {
  opacity: 0.55;
}
</style>

<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { cn } from "@/lib/utils";
import { Check, ChevronsUpDown, Loader2, RefreshCw } from "lucide-vue-next";

interface AppConfig {
  hotkeyDictate: string;
  hotkeyCommand: string;
  polishMode: string;
  asrModel: string;
  llmModel: string;
  maxRecordingSeconds: number;
  inputDevice?: string | null;
}

const onboarded = ref(false);
const showKeyForm = ref(false);
const apiKey = ref("");
const saving = ref(false);
const message = ref("");
const error = ref("");
const config = ref<AppConfig | null>(null);

const modelOpen = ref(false);
const modelSearch = ref("");
const models = ref<string[]>([]);
const modelsLoading = ref(false);
const modelsError = ref("");
const savingModel = ref(false);
const modelMessage = ref("");

// 录音设备下拉：打开时枚举，选中即保存；null 表示跟随系统默认
const deviceOpen = ref(false);
const devices = ref<string[]>([]);
const devicesLoading = ref(false);
const devicesError = ref("");

// 最长录音秒数编辑框：回车或失焦保存，越界自动钳位到 5 到 1200
const maxSeconds = ref("");
const savingMaxSeconds = ref(false);

onMounted(async () => {
  try {
    const status = await invoke<{ onboarded: boolean; config: AppConfig }>("get_setup_status");
    onboarded.value = status.onboarded;
    config.value = status.config;
    maxSeconds.value = String(status.config.maxRecordingSeconds ?? 60);
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

async function loadModels(force = false) {
  if (modelsLoading.value) return;
  if (!force && models.value.length > 0) return;
  modelsLoading.value = true;
  modelsError.value = "";
  try {
    models.value = await invoke<string[]>("list_llm_models");
    if (models.value.length === 0) {
      modelsError.value = "账号下暂无 qwen 系列模型，可直接输入模型名使用";
    }
  } catch (e) {
    modelsError.value = `${String(e)}。可直接输入模型名使用`;
  } finally {
    modelsLoading.value = false;
  }
}

async function loadDevices() {
  if (devicesLoading.value) return;
  devicesLoading.value = true;
  devicesError.value = "";
  try {
    devices.value = await invoke<string[]>("list_input_devices");
  } catch (e) {
    devicesError.value = String(e);
  } finally {
    devicesLoading.value = false;
  }
}

function onDeviceOpenChange(open: boolean | undefined) {
  deviceOpen.value = open ?? false;
  if (open) {
    devicesError.value = "";
    void loadDevices();
  }
}

async function chooseDevice(device: string | null) {
  deviceOpen.value = false;
  const current = config.value?.inputDevice ?? null;
  if (device === current) return;
  await invoke("set_input_device", { device });
  if (config.value) config.value.inputDevice = device;
}

const filteredModels = computed(() => {
  const q = modelSearch.value.trim().toLowerCase();
  if (!q) return models.value;
  return models.value.filter((m) => m.toLowerCase().includes(q));
});

// 搜索词不是列表中的模型时，允许作为自定义模型名直接使用
const customModel = computed(() => {
  const q = modelSearch.value.trim();
  if (!q || models.value.includes(q)) return "";
  return q;
});

function onModelOpenChange(open: boolean | undefined) {
  modelOpen.value = open ?? false;
  if (open) {
    modelSearch.value = "";
    modelMessage.value = "";
    void loadModels();
  }
}

async function chooseModel(model: string) {
  modelOpen.value = false;
  if (model === config.value?.llmModel) return;
  savingModel.value = true;
  error.value = "";
  modelMessage.value = "";
  try {
    await invoke("set_llm_model", { model });
    if (config.value) config.value.llmModel = model;
    modelMessage.value = "润色模型已更新";
  } catch (e) {
    error.value = String(e);
  } finally {
    savingModel.value = false;
  }
}

function onModelSearchEnter() {
  if (filteredModels.value.length === 1) {
    void chooseModel(filteredModels.value[0]);
  } else if (customModel.value) {
    void chooseModel(customModel.value);
  }
}

const savingPolishMode = ref(false);

async function choosePolishMode(mode: "raw" | "polished") {
  if (savingPolishMode.value || config.value?.polishMode === mode) return;
  savingPolishMode.value = true;
  error.value = "";
  try {
    await invoke("set_polish_mode", { mode });
    if (config.value) config.value.polishMode = mode;
  } catch (e) {
    error.value = String(e);
  } finally {
    savingPolishMode.value = false;
  }
}

function segmentedClass(active: boolean) {
  return cn(
    "cursor-pointer rounded-md px-3 py-1 text-sm transition-colors",
    active
      ? "bg-background font-medium text-foreground shadow-sm"
      : "text-muted-foreground hover:text-foreground",
  );
}

async function saveMaxSeconds() {
  if (savingMaxSeconds.value) return;
  const parsed = Math.round(Number(maxSeconds.value));
  if (!Number.isFinite(parsed)) {
    maxSeconds.value = String(config.value?.maxRecordingSeconds ?? 60);
    return;
  }
  const n = Math.min(1200, Math.max(5, parsed));
  maxSeconds.value = String(n);
  if (n === config.value?.maxRecordingSeconds) return;
  savingMaxSeconds.value = true;
  try {
    await invoke("set_max_recording_seconds", { seconds: n });
    if (config.value) config.value.maxRecordingSeconds = n;
  } catch (e) {
    error.value = String(e);
  } finally {
    savingMaxSeconds.value = false;
  }
}
</script>

<template>
  <main class="flex min-h-screen flex-col items-center justify-center gap-3 p-6">
    <h1 class="text-2xl font-semibold tracking-tight">SimpleLess</h1>
    <p class="mb-2 text-sm text-muted-foreground">语音输入工具</p>

    <section
      v-if="!onboarded || showKeyForm"
      class="flex w-full max-w-md flex-col gap-4 rounded-xl border bg-card p-5 text-card-foreground shadow-sm"
    >
      <div class="flex flex-col gap-1">
        <h2 class="text-base font-semibold">{{ onboarded ? "更换 API Key" : "初次使用" }}</h2>
        <p v-if="!onboarded" class="text-sm leading-relaxed text-muted-foreground">
          填入阿里云百炼平台的 API Key。
        </p>
      </div>
      <div class="flex flex-col gap-2">
        <Input
          v-model="apiKey"
          type="password"
          placeholder="sk-..."
          spellcheck="false"
          @keyup.enter="save"
        />
        <div class="flex gap-2">
          <Button :disabled="saving || !apiKey.trim()" class="flex-1" @click="save">
            <Loader2 v-if="saving" class="animate-spin" />
            {{ saving ? "正在验证..." : "验证并保存" }}
          </Button>
          <Button v-if="onboarded" variant="outline" @click="showKeyForm = false">取消</Button>
        </div>
      </div>
      <p v-if="error" class="text-sm text-destructive">{{ error }}</p>
    </section>

    <section
      v-else
      class="flex w-full max-w-md flex-col gap-4 rounded-xl border bg-card p-5 text-card-foreground shadow-sm"
    >
      <h2 class="text-base font-semibold">已就绪</h2>
      <p v-if="message" class="text-sm text-green-600 dark:text-green-400">{{ message }}</p>
      <p class="text-xs leading-relaxed text-muted-foreground">
        按听写热键开始、再按一次结束，文本会插入当前光标处；按命令热键后用语音修改设置，比如说“切换成原文”。
      </p>

      <div class="flex flex-col gap-2">
        <h3 class="text-xs font-medium tracking-wider text-muted-foreground">录音与润色</h3>
        <div class="flex flex-col gap-3 rounded-lg border bg-background p-3">
          <div class="flex flex-col gap-1.5">
            <div class="flex items-center justify-between gap-3">
              <div class="flex items-center gap-1">
                <span class="text-sm font-medium">润色模型</span>
                <Button
                  variant="ghost"
                  size="icon"
                  class="size-6"
                  title="重新获取模型列表"
                  :disabled="modelsLoading"
                  @click="loadModels(true)"
                >
                  <RefreshCw :class="cn('size-3.5', modelsLoading && 'animate-spin')" />
                </Button>
              </div>
              <div class="flex items-center gap-1.5">
                <Popover :open="modelOpen" @update:open="onModelOpenChange">
                  <PopoverTrigger as-child>
                    <Button
                      variant="outline"
                      role="combobox"
                      :aria-expanded="modelOpen"
                      class="w-44 justify-between font-normal"
                    >
                      <span class="truncate font-mono text-xs">{{
                        config?.llmModel ?? "选择模型"
                      }}</span>
                      <ChevronsUpDown class="size-4 shrink-0 opacity-50" />
                    </Button>
                  </PopoverTrigger>
                  <PopoverContent align="start" class="w-[var(--reka-popover-trigger-width)] p-0">
                    <div class="flex h-9 items-center gap-2 border-b px-3">
                      <Input
                        v-model="modelSearch"
                        placeholder="搜索或输入模型名..."
                        class="h-9 border-0 px-0 shadow-none focus-visible:ring-0"
                        @keyup.enter="onModelSearchEnter"
                      />
                    </div>
                    <div class="max-h-60 overflow-y-auto p-1">
                      <div
                        v-if="modelsLoading"
                        class="flex items-center justify-center gap-2 px-2 py-6 text-sm text-muted-foreground"
                      >
                        <Loader2 class="size-4 animate-spin" /> 正在获取模型列表...
                      </div>
                      <template v-else>
                        <p
                          v-if="modelsError && models.length === 0"
                          class="px-2 py-4 text-xs leading-relaxed text-muted-foreground"
                        >
                          {{ modelsError }}
                        </p>
                        <button
                          v-for="m in filteredModels"
                          :key="m"
                          type="button"
                          class="flex w-full cursor-pointer items-center justify-between gap-2 rounded-sm px-2 py-1.5 text-left font-mono text-sm hover:bg-accent"
                          @click="chooseModel(m)"
                        >
                          <span class="truncate">{{ m }}</span>
                          <Check
                            v-if="m === config?.llmModel"
                            class="size-4 shrink-0 text-primary"
                          />
                        </button>
                        <button
                          v-if="customModel"
                          type="button"
                          class="flex w-full cursor-pointer items-center gap-2 rounded-sm px-2 py-1.5 text-left text-sm hover:bg-accent"
                          @click="chooseModel(customModel)"
                        >
                          使用自定义模型
                          <span class="truncate font-mono text-xs text-muted-foreground">
                            {{ customModel }}
                          </span>
                        </button>
                        <p
                          v-if="models.length > 0 && filteredModels.length === 0 && !customModel"
                          class="px-2 py-4 text-center text-sm text-muted-foreground"
                        >
                          无匹配模型
                        </p>
                      </template>
                    </div>
                  </PopoverContent>
                </Popover>
              </div>
            </div>
            <p v-if="savingModel" class="text-xs text-muted-foreground">正在保存...</p>
            <p v-if="modelMessage" class="text-xs text-green-600 dark:text-green-400">
              {{ modelMessage }}
            </p>
          </div>

          <div class="flex items-center justify-between gap-3">
            <span class="text-sm font-medium">润色档位</span>
            <div class="inline-flex items-center gap-0.5 rounded-lg bg-muted p-1">
              <button
                type="button"
                :class="segmentedClass(config?.polishMode !== 'polished')"
                @click="choosePolishMode('raw')"
              >
                原文
              </button>
              <button
                type="button"
                :class="segmentedClass(config?.polishMode === 'polished')"
                @click="choosePolishMode('polished')"
              >
                润色
              </button>
            </div>
          </div>

          <div class="flex flex-col gap-1.5">
            <div class="flex items-center justify-between gap-3">
              <span class="text-sm font-medium">最长录音</span>
              <div class="flex items-center gap-1.5">
                <Input
                  v-model="maxSeconds"
                  type="number"
                  min="5"
                  max="1200"
                  :disabled="savingMaxSeconds"
                  class="h-8 w-20 text-center [&::-webkit-inner-spin-button]:appearance-none [&::-webkit-outer-spin-button]:appearance-none"
                  @keyup.enter="saveMaxSeconds"
                  @blur="saveMaxSeconds"
                />
                <span class="text-xs text-muted-foreground">秒</span>
              </div>
            </div>
          </div>

          <div class="flex items-center justify-between gap-3">
            <span class="text-sm font-medium">录音设备</span>
            <Popover :open="deviceOpen" @update:open="onDeviceOpenChange">
              <PopoverTrigger as-child>
                <Button
                  variant="outline"
                  role="combobox"
                  :aria-expanded="deviceOpen"
                  class="w-44 justify-between font-normal"
                >
                  <span class="truncate text-xs">{{ config?.inputDevice ?? "系统默认" }}</span>
                  <ChevronsUpDown class="size-4 shrink-0 opacity-50" />
                </Button>
              </PopoverTrigger>
              <PopoverContent align="start" class="w-[var(--reka-popover-trigger-width)] p-0">
                <div class="max-h-60 overflow-y-auto p-1">
                  <div
                    v-if="devicesLoading"
                    class="flex items-center justify-center gap-2 px-2 py-6 text-sm text-muted-foreground"
                  >
                    <Loader2 class="size-4 animate-spin" /> 正在获取录音设备...
                  </div>
                  <p
                    v-else-if="devicesError"
                    class="px-2 py-4 text-xs leading-relaxed text-muted-foreground"
                  >
                    {{ devicesError }}
                  </p>
                  <template v-else>
                    <button
                      type="button"
                      class="flex w-full cursor-pointer items-center justify-between gap-2 rounded-sm px-2 py-1.5 text-left text-sm hover:bg-accent"
                      @click="chooseDevice(null)"
                    >
                      <span>系统默认</span>
                      <Check
                        v-if="!config?.inputDevice"
                        class="size-4 shrink-0 text-primary"
                      />
                    </button>
                    <button
                      v-for="d in devices"
                      :key="d"
                      type="button"
                      class="flex w-full cursor-pointer items-center justify-between gap-2 rounded-sm px-2 py-1.5 text-left text-sm hover:bg-accent"
                      @click="chooseDevice(d)"
                    >
                      <span class="truncate">{{ d }}</span>
                      <Check
                        v-if="d === config?.inputDevice"
                        class="size-4 shrink-0 text-primary"
                      />
                    </button>
                    <p v-if="devices.length === 0" class="px-2 py-4 text-center text-sm text-muted-foreground">
                      未发现录音设备
                    </p>
                  </template>
                </div>
              </PopoverContent>
            </Popover>
          </div>
        </div>
      </div>

      <div class="flex flex-col gap-2">
        <h3 class="text-xs font-medium tracking-wider text-muted-foreground">热键</h3>
        <dl v-if="config" class="flex flex-col gap-2 rounded-lg bg-muted/50 p-3 text-sm">
          <div class="flex justify-between">
            <dt class="text-muted-foreground">听写热键</dt>
            <dd class="font-mono">{{ config.hotkeyDictate }}</dd>
          </div>
          <div class="flex justify-between">
            <dt class="text-muted-foreground">命令热键</dt>
            <dd class="font-mono">{{ config.hotkeyCommand }}</dd>
          </div>
          <div class="flex justify-between">
            <dt class="text-muted-foreground">识别模型</dt>
            <dd class="font-mono">{{ config.asrModel }}</dd>
          </div>
        </dl>
      </div>

    </section>

    <Button variant="outline" @click="showKeyForm = true">更换 API Key</Button>
  </main>
</template>

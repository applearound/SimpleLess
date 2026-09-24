<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
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
  asrEngine: "cloud" | "local";
  llmModel: string;
  maxRecordingSeconds: number;
  inputDevice?: string | null;
}

type LocalModelStatus = {
  state: "not_downloaded" | "downloading" | "failed" | "ready";
  downloaded?: number;
  total?: number;
  error?: string;
};

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

// 本地识别模型状态与下载进度，事件由后端推送
const localModel = ref<LocalModelStatus>({ state: "not_downloaded" });
const switchingEngine = ref(false);
// 下载完成后自动切到本地引擎的意愿标记：点击本地行触发下载时置真，
// 下载期间用户主动点回云端则作废
const autoSwitchOnReady = ref(false);
let unlistenProgress: (() => void) | null = null;
let unlistenState: (() => void) | null = null;
let unlistenFocus: (() => void) | null = null;

const isLocal = computed(() => config.value?.asrEngine === "local");

const downloadPercent = computed(() => {
  const { downloaded = 0, total = 0 } = localModel.value;
  if (!total) return 0;
  return Math.min(100, Math.round((downloaded / total) * 100));
});

function fmtBytes(n: number): string {
  if (!n) return "0MB";
  const mb = n / (1024 * 1024);
  return mb >= 100 ? `${Math.round(mb)}MB` : `${mb.toFixed(1)}MB`;
}

onMounted(async () => {
  try {
    const status = await invoke<{ onboarded: boolean; config: AppConfig }>("get_setup_status");
    onboarded.value = status.onboarded;
    config.value = status.config;
    maxSeconds.value = String(status.config.maxRecordingSeconds ?? 60);
  } catch (e) {
    error.value = String(e);
  }
  try {
    localModel.value = await invoke<LocalModelStatus>("get_local_model_status");
  } catch {
    // 状态探测失败不阻断设置页，按未下载处理
  }
  unlistenProgress = await listen<{ downloaded: number; total: number }>(
    "local-model-progress",
    (e) => {
      localModel.value = { state: "downloading", ...e.payload };
    },
  );
  unlistenState = await listen<{ state: string; error?: string }>("local-model-state", async (e) => {
    if (e.payload.state === "ready") {
      localModel.value = { state: "ready" };
      if (autoSwitchOnReady.value) {
        autoSwitchOnReady.value = false;
        await applyEngine("local");
      }
    } else if (e.payload.state === "failed") {
      localModel.value = { state: "failed", error: e.payload.error ?? "下载失败" };
      autoSwitchOnReady.value = false;
    } else {
      // 取消：回到探测到的真实状态
      try {
        localModel.value = await invoke<LocalModelStatus>("get_local_model_status");
      } catch {
        localModel.value = { state: "not_downloaded" };
      }
    }
  });
  // 设置窗口隐藏后复用，组件不重新挂载；窗口每次重新聚焦时
  // 重新探测模型状态，文件可能被外部手动增删
  unlistenFocus = await getCurrentWindow().onFocusChanged(({ payload: focused }) => {
    if (!focused || localModel.value.state === "downloading") return;
    invoke<LocalModelStatus>("get_local_model_status")
      .then((s) => {
        localModel.value = s;
      })
      .catch(() => {});
  });
});

onUnmounted(() => {
  unlistenProgress?.();
  unlistenState?.();
  unlistenFocus?.();
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

async function applyEngine(engine: "cloud" | "local") {
  if (switchingEngine.value) return;
  switchingEngine.value = true;
  error.value = "";
  try {
    await invoke("set_asr_engine", { engine });
    if (config.value) config.value.asrEngine = engine;
  } catch (e) {
    error.value = String(e);
  } finally {
    switchingEngine.value = false;
  }
}

async function chooseEngine(engine: "cloud" | "local") {
  if (engine === "cloud") {
    autoSwitchOnReady.value = false;
    if (config.value?.asrEngine !== "cloud") await applyEngine("cloud");
    return;
  }
  if (config.value?.asrEngine === "local") return;
  if (localModel.value.state === "ready") {
    await applyEngine("local");
    return;
  }
  if (localModel.value.state === "downloading") {
    // 下载中再次点击视为确认下载完成后自动切换
    autoSwitchOnReady.value = true;
    return;
  }
  autoSwitchOnReady.value = true;
  try {
    await invoke("download_local_model");
    localModel.value = { state: "downloading", downloaded: 0, total: 0 };
  } catch (e) {
    error.value = String(e);
    autoSwitchOnReady.value = false;
  }
}

async function cancelDownload() {
  try {
    await invoke("cancel_local_model_download");
  } catch (e) {
    error.value = String(e);
  }
}

async function deleteLocalModel() {
  error.value = "";
  try {
    await invoke("delete_local_model");
    localModel.value = { state: "not_downloaded" };
    if (config.value?.asrEngine === "local") await applyEngine("cloud");
  } catch (e) {
    error.value = String(e);
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
        <h3 class="text-xs font-medium tracking-wider text-muted-foreground">语音识别</h3>
        <div class="flex flex-col gap-1.5 rounded-lg border bg-background p-3">
          <button
            type="button"
            class="flex w-full cursor-pointer items-start justify-between gap-3 rounded-md p-2 text-left transition-colors hover:bg-accent"
            :class="isLocal && 'opacity-50'"
            :title="isLocal ? '点击切回云端识别' : undefined"
            @click="chooseEngine('cloud')"
          >
            <div class="flex flex-col gap-0.5">
              <div class="flex items-center gap-2">
                <span class="text-sm font-medium">云端识别</span>
                <span
                  v-if="isLocal"
                  class="rounded-full bg-muted px-2 py-0.5 text-[10px] leading-4 text-muted-foreground"
                >
                  当前使用本地模型
                </span>
              </div>
              <p class="text-xs leading-relaxed text-muted-foreground">
                百炼实时识别，需联网与 API Key
              </p>
            </div>
            <Check v-if="!isLocal" class="mt-0.5 size-4 shrink-0 text-primary" />
          </button>

          <div
            v-if="localModel.state === 'downloading'"
            class="flex flex-col gap-2 rounded-md bg-accent/40 p-2"
          >
            <div class="flex items-center justify-between gap-3">
              <div class="flex flex-col gap-0.5">
                <span class="text-sm font-medium">本地识别</span>
                <p class="text-xs text-muted-foreground">
                  正在下载 {{ fmtBytes(localModel.downloaded ?? 0) }} /
                  {{ fmtBytes(localModel.total ?? 0) }}
                </p>
              </div>
              <Button variant="outline" size="sm" @click="cancelDownload">取消</Button>
            </div>
            <div class="h-1.5 w-full overflow-hidden rounded-full bg-muted">
              <div
                class="h-full rounded-full bg-primary transition-all"
                :style="{ width: `${downloadPercent}%` }"
              />
            </div>
          </div>

          <button
            v-else
            type="button"
            class="flex w-full cursor-pointer items-start justify-between gap-3 rounded-md p-2 text-left transition-colors hover:bg-accent"
            @click="chooseEngine('local')"
          >
            <div class="flex flex-col gap-0.5">
              <div class="flex items-center gap-2">
                <span class="text-sm font-medium">本地识别</span>
                <Check v-if="isLocal" class="size-4 text-primary" />
              </div>
              <p v-if="localModel.state === 'failed'" class="text-xs leading-relaxed text-destructive">
                {{ localModel.error ?? "下载失败" }}，点击重试
              </p>
              <p v-else class="text-xs leading-relaxed text-muted-foreground">
                {{
                  localModel.state === "ready"
                    ? "内置离线小模型已就绪，中英日韩粤"
                    : "内置离线小模型，中英日韩粤，约 230MB，点击下载"
                }}
              </p>
            </div>
            <Loader2 v-if="switchingEngine" class="mt-0.5 size-4 shrink-0 animate-spin" />
          </button>

          <div v-if="localModel.state === 'ready'" class="flex justify-end px-2 pb-1">
            <button
              type="button"
              class="cursor-pointer text-xs text-muted-foreground underline-offset-2 hover:text-foreground hover:underline"
              @click="deleteLocalModel"
            >
              删除本地模型，释放空间
            </button>
          </div>
        </div>
      </div>

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
            <dd class="font-mono" :class="isLocal && 'text-muted-foreground'">
              {{ isLocal ? "sense-voice（本地）" : config.asrModel }}
            </dd>
          </div>
        </dl>
      </div>

    </section>

    <Button variant="outline" @click="showKeyForm = true">更换 API Key</Button>
  </main>
</template>

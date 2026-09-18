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

onMounted(async () => {
  try {
    const status = await invoke<{ onboarded: boolean; config: AppConfig }>("get_setup_status");
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
          填入阿里云百炼平台的 API Key，这是唯一一次需要键盘的配置。验证通过后，所有操作都通过语音完成。
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

      <div class="flex flex-col gap-1.5">
        <span class="text-sm font-medium">润色模型</span>
        <div class="flex items-center gap-1.5">
          <Popover :open="modelOpen" @update:open="onModelOpenChange">
            <PopoverTrigger as-child>
              <Button
                variant="outline"
                role="combobox"
                :aria-expanded="modelOpen"
                class="flex-1 justify-between font-normal"
              >
                <span class="truncate font-mono text-xs">{{ config?.llmModel ?? "选择模型" }}</span>
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
                    <Check v-if="m === config?.llmModel" class="size-4 shrink-0 text-primary" />
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
          <Button
            variant="ghost"
            size="icon"
            title="重新获取模型列表"
            :disabled="modelsLoading"
            @click="loadModels(true)"
          >
            <RefreshCw :class="cn('size-4', modelsLoading && 'animate-spin')" />
          </Button>
        </div>
        <p v-if="savingModel" class="text-xs text-muted-foreground">正在保存...</p>
        <p v-if="modelMessage" class="text-xs text-green-600 dark:text-green-400">
          {{ modelMessage }}
        </p>
      </div>

      <div class="flex flex-col gap-1.5">
        <span class="text-sm font-medium">润色档位</span>
        <div class="inline-flex w-fit items-center gap-0.5 rounded-lg bg-muted p-1">
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

      <dl v-if="config" class="flex flex-col gap-1.5 text-sm">
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

      <p class="text-xs leading-relaxed text-muted-foreground">
        按听写热键开始、再按一次结束，文本会插入当前光标处；按命令热键后用语音修改设置，比如说“切换成原文”。
      </p>
      <Button variant="outline" class="self-start" @click="showKeyForm = true">更换 API Key</Button>
    </section>
  </main>
</template>

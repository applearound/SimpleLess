# 本地语音识别引擎实施计划

前置调研见 docs/local-asr-models.md。本计划为可直接开工的细化版本。

已确认的产品决策：

1. 设置页新增语音识别区块，提供云端百炼与本地内置两种引擎选择。
2. 勾选本地模型且模型未下载时，立即开始下载，下载完成自动启用本地识别。
3. 启用本地后，在线识别那一项整体置灰不可选，界面显示当前使用的是本地模型。
4. 润色区块（模型与档位）不随引擎开关变化，仍按是否配置 API Key 决定可用性。

## 配置改动（config.rs）

AppConfig 新增字段：

```rust
#[serde(default)]
pub asr_engine: AsrEngine,
```

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AsrEngine { Cloud, Local }
```

默认 Cloud，旧 config.json 无该字段时 serde 自动取默认值，升级无感。`asr_model` 与 `language_hints` 保留，仅云端引擎使用。

本地模型状态不写入 config.json，由 app data 目录探测决定，避免配置与磁盘不一致。

## 模型文件布局

```
{app_data_dir}/models/sense-voice/
  model.int8.onnx    约 228MB
  tokens.txt         约 300KB
  silero_vad.onnx    约 2MB
  meta.json          版本与 sha256 记录
```

下载按单文件直链进行，不走压缩包，免去解压依赖。每个文件先写 `.part` 临时名，sha256 校验通过后改名，三个文件全部就位后写 meta.json，此后判定为 Ready。

下载源按顺序回落，写成一个 URL 数组：

1. hf-mirror.com 的 HuggingFace 镜像直链
2. huggingface.co 原始直链
3. GitHub k2-fsa/sherpa-onnx releases 单文件直链

ModelScope 直链仓库名在实现时核实后可插到首位，境内速度最快。

## 后端模块

### asr_local.rs（新）

`pub fn start(model_dir: &Path) -> Result<AsrSession, String>`，直接复用 asr.rs 里的 AsrSession 类型，三通道语义不变：

- audio_tx 接收 16kHz 单声道 i16 块，独立 std::thread 内转 f32 累积并喂 Silero VoiceActivityDetector
- VAD 切出语音段后立即用 SenseVoice OfflineRecognizer 识别，结果追加推 partial_tx，实现字幕条伪流式
- audio_tx 全部克隆体 drop 后，处理尾段并经 done_tx 返回全文，与云端会话的结束语义一致
- onnxruntime 阻塞调用全部在该线程，不进 async 运行时

### model_store.rs（新）

- `probe()`：检查模型目录与 meta.json，返回 NotDownloaded / Ready / Corrupt
- `download()`：全局单任务，reqwest 流式下载，进度经 Tauri 事件 `local-model-progress`（已下载字节数与总字节数）推送，终态经 `local-model-state` 推送
- `cancel()`：中止当前任务，保留 `.part` 文件，重试时用 Range 续传（可选做，重新下载也可接受）
- `delete()`：删除模型目录释放空间
- sha256 校验，Cargo 新增 sha2 依赖

### pipeline.rs

当前 asr::start 调用点（约 250 行）改为按 cfg.asr_engine 分发：

- Cloud：现状不变，需要 API Key
- Local：走 asr_local::start，不取 Key；启动前 probe 模型，缺失时返回错误并提示已回退云端或重新下载

录音会话进行中修改引擎配置不影响当前会话，下一次按热键生效。

## Tauri 命令与事件

命令：

- `get_asr_engine_status`：返回当前引擎与本地模型状态（含下载进度）
- `set_asr_engine(engine)`：切 Local 时要求模型 Ready，否则报错
- `download_local_model` / `cancel_local_model_download`
- `delete_local_model`

事件：

- `local-model-progress`：`{ downloaded, total }`
- `local-model-state`：`{ state: "ready" | "failed" | "cancelled" }`

## 设置页 UI（App.vue）

在录音与润色分组上方新增语音识别分组，两个单选项：

- 云端识别：副文案为百炼实时识别，需联网与 API Key。本地引擎启用时该行整体置灰（透明度加禁用样式），并在分组标题旁显示当前使用本地模型的徽标提示
- 本地识别：副文案为内置离线小模型，支持中英日韩粤，约 230MB。状态分四种展示：
  - 未下载：显示下载模型按钮
  - 下载中：进度条加已下载大小与百分比，提供取消
  - 失败：错误信息加重试按钮
  - 就绪：选中态即生效

润色模型下拉与档位保持现有逻辑，仅按 API Key 有无控制可用性，不随引擎开关变灰。

## 实施步骤

按四个独立可验证的提交推进：

1. 本地引擎打通：Cargo 加 sherpa-onnx 与 sha2，写 asr_local.rs，pipeline 分发，手工放置模型文件，本机验证本地听写与字幕条伪流式全流程
2. 下载器与命令：model_store.rs，注册命令与事件，验证下载、校验、取消、续跑
3. 设置页 UI：语音识别分组、下载进度、置灰逻辑与提示徽标
4. 打包验证：bun run build:quick 与 build:dist 走通，确认 sherpa-onnx 静态链接在打包脚本下无问题；macOS 侧需 cmake 存在，验证清单留给对应环境

## 验证清单

- bun run test:rust 全量通过
- 手工场景：
  - 切本地引擎，热键听写，字幕条逐段上屏，松键后全文进入润色，插入光标处
  - 删除 API Key，本地识别加原文档位完整可用；润色与命令模式给出缺 Key 提示
  - 下载中断后重试可完成；模型文件删除后按热键给出明确错误提示
  - 切回云端引擎一切如旧

## 风险与备注

- sherpa-onnx-sys 静态构建需要 cmake，Windows 本机已确认 cmake 4.4.3 存在；macOS 需安装 cmake
- 静态链接 onnxruntime 会使安装包增大约 10 到 20MB
- int8 量化精度对中文日常听写够用，如后续反馈识别偏差，可增加 fp32（894MB）可选项
- SenseVoice 伪流式的字幕延迟取决于说话停顿，正常语速约 0.5 到 2 秒一段

## 实现记录（2026-09-25）

Windows 侧已全部实现并验证，与计划的偏差如下：

- 下载源实际为 hf-mirror 优先、HuggingFace 与 GitHub 依次回落。ModelScope 未找到对应仓库的稳定直链，未纳入。三个文件的 sha256 已核实并硬编码进下载器做完整性校验
- meta.json 未实现：临时文件加 sha256 校验加原子改名已覆盖完整性需求，就绪判定直接探测三个文件是否齐备
- 断点续传未实现：取消下载即删除半成品，重新下载完整文件
- 置灰交互：启用本地后云端识别行半透明置灰并显示当前使用本地模型徽标，但保留点击切回能力，未做硬禁用
- 下载完成后自动切换到本地引擎的逻辑在前端实现：下载由点击本地行触发，完成后若用户未中途点回云端则自动生效

验证结果：

- cargo test 全量通过；本地识别集成测试用官方 zh.wav 跑通，识别文本为"开饭时间早上9点至下午5点。"，ITN 数字规整与句尾标点正常，含模型加载全程 2.65 秒
- bun run build:quick 通过，release 编译 2 分钟，exe 体积 25MB
- 待人工验证：GUI 内热键听写与字幕条伪流式效果；macOS 侧构建（需 cmake）


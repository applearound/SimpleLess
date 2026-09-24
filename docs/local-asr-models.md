# 本地中英双语 ASR 小模型调研

背景：当前语音识别走百炼 `fun-asr-realtime` WebSocket 双工会话。设想新增一种内置本地小模型，离线完成基础识别，润色仍交千问大模型。本文调研支持中英双语的本地小模型方案。调研时间 2026-09-25。

## 结论

方案存在且成熟。推荐主选 sherpa-onnx 运行时加 SenseVoiceSmall int8 模型。体积 228MB，桌面 CPU 实时率约 0.1，即 10 秒录音约 1 秒内出全文。中英日韩粤五语言，自带标点与数字规整。上游模型 MIT 许可，运行时 Apache-2.0，可商用可内置。

## 候选对比

| 方案 | 体积 | 语言 | 速度(CPU) | 流式 | 标点 | 许可 |
|---|---|---|---|---|---|---|
| SenseVoiceSmall int8 | 228MB | 中英日韩粤 | RTF 约 0.1 | 非流式，VAD 分段伪流式 | 有，含 ITN | MIT |
| Qwen3-ASR-0.6B int8 | 约 940MB | 52 语言 | 社区实测 RTF 约 0.7 | 非流式 | 有 | Apache-2.0 |
| Zipformer bilingual zh-en int8 | 约 230MB | 中英 | 快，真流式 | 是 | 无 | Apache-2.0 |
| Whisper small (whisper.cpp) | 466MB | 多语言 | RTF 约 0.5 到 1 | 弱，切窗口模拟 | 无 | MIT |

各候选细节：

### SenseVoiceSmall（推荐）

阿里 FunAudioLLM 2024 年发布的中小型 ASR，sherpa-onnx 官方维护转换后的 ONNX 版本。

- 下载：GitHub k2-fsa/sherpa-onnx releases 的 asr-models 分类，包名 `sherpa-onnx-sense-voice-zh-en-ja-ko-yue-int8-2024-07-17.tar.bz2`，主文件 `model.int8.onnx` 228MB 加 `tokens.txt`。fp32 版 894MB 可作高精度备选。
- 官方实测 RTF：macOS 单线程 0.109，RK3588 Cortex-A76 四线程 0.049。桌面 CPU 更快。
- 中英效果：SenseVoice 论文给出与 Whisper-Large-v3 相当或更好的中英日韩准确率，速度高一个数量级。中英混说支持良好。
- 输出带标点，`use_itn` 开关控制数字日期规整，中文听写体验直接可用。
- 2025-09-09 有粤语增强微调版，中文普通话场景用 2024-07-17 原版即可。

### Qwen3-ASR-0.6B（高精度备选，不内置）

Qwen 于 2026 年 1 月开源的 LLM 架构 ASR，52 语言，0.6B 与 1.7B 两个尺寸。sherpa-onnx 已支持其 int8 ONNX 版本。

- HF 仓库 `csukuangfj2/sherpa-onnx-qwen3-asr-0.6B-int8-2026-03-25`：decoder 721MB，encoder 174MB，前端 44MB，合计约 940MB。
- LLM 解码器逐 token 生成，CPU 上延迟高。社区 ONNX CPU 实测 RTF 约 0.71，10 分钟音频峰值内存 5.7GB（fp32 版数据，int8 好于该值但仍重）。GPU 加 vLLM 时吞吐极高，桌面 CPU 场景不合适做默认内置。
- 定位：机器强、要最高精度的用户可后续做成可选项。

### 流式 Zipformer bilingual zh-en（字幕条备选）

`sherpa-onnx-streaming-zipformer-bilingual-zh-en-2023-02-20`，真流式 transducer，encoder int8 182MB，整包约 230MB。有 small 变体更小。

- 优点：真流式，逐段出 partial，与现有字幕条交互模型一致。
- 缺点：transducer 输出无标点，中英混说准确率低于 SenseVoice，模型较老（2023-02）。

### Whisper（不推荐首选）

whisper.cpp 与 whisper-rs 生态成熟。但中英场景下 tiny 与 base 中文错字多，small 起步可用且 466MB，同体积下速度与中文质量都低于 SenseVoice，small 档无标点输出。流式只能滑动窗口模拟。综合不占优。

## 运行时集成

sherpa-onnx 官方 Rust crate 已发布 crates.io：

- 包名 `sherpa-onnx`，版本 1.13.8（2026-09-11 更新），k2-fsa 维护者本人发布，累计下载约 44 万，Apache-2.0。
- 默认 feature 为 `static`，由 `sherpa-onnx-sys` 静态链接 C++ 库与 onnxruntime，产出单文件二进制，符合 Tauri 打包形态。社区旧绑定 sherpa-rs 已废弃，不必考虑。
- crate 提供 `OfflineRecognizer`、`OnlineRecognizer`、`VoiceActivityDetector`、`LinearResampler`、`DisplayManager` 等类型。官方 `rust-api-examples` 含 `sense_voice_simulate_streaming_microphone.rs`，演示 Silero VAD 检测语音段后逐段跑离线 SenseVoice，实现伪流式，连 cpal 麦克风采集都一并示范。

对接现有代码的路径：

1. `Cargo.toml` 加 `sherpa-onnx = "1.13.8"`。构建机需要 C++ 编译链与 cmake，Windows 走 MSVC，macOS 走 clang，Tauri 构建环境本就具备，CI 需验证。
2. 新建 `asr_local.rs`，复刻 `asr.rs` 的 `AsrSession` 三通道接口：`audio_tx` 收 16kHz i16 PCM 转 f32 喂 VAD，每个语音段识别完推 `partial_tx`，发送端全部 drop 后把剩余音频识别完经 `done_tx` 返回全文。`pipeline.rs` 与字幕条无需改动。
3. 推理放独立线程，onnxruntime 阻塞调用不进 async 运行时。
4. 模型文件放 app data 目录，首启向导或设置页触发下载。SenseVoice int8 228MB 加 Silero VAD 约 2MB。下载源优先 ModelScope 或 hf-mirror.com，境内速度快；GitHub releases 与 HuggingFace 作 fallback。

## 字幕条流式问题

现有字幕条消费 `partial_rx` 实时文本。SenseVoice 为非流式模型，两种处理方式：

- 伪流式：Silero VAD 按语音间隙分段，每段结束立刻出该段文本追加到 partial。正常语速下停顿间隔 0.5 到 2 秒，字幕延迟可接受。官方示例即此方案。
- 真流式：再引入流式 zipformer 约 230MB，partial 即时但无标点。成本翻倍，先不做。

按热键说话的使用形态下，用户松键前的字幕延迟体验影响有限，先做伪流式。

## 许可与分发

- sherpa-onnx 运行时：Apache-2.0。
- SenseVoiceSmall 模型：上游 FunAudioLLM/SenseVoice 为 MIT（LICENSE 文件已核实），sherpa 转换包内附 LICENSE。
- Zipformer 模型：Apache-2.0。
- 均可商用与随应用分发。模型不打进安装包，按需下载，安装包体积不膨胀。

## 落地建议

1. 第一期：本地引擎接 SenseVoiceSmall int8 加 Silero VAD，实现 `AsrSession` 同构接口，配置里加识别引擎选项（在线千问或本地内置）。
2. 第二期视反馈：加 Qwen3-ASR-0.6B 高精度可选下载；或加流式 zipformer 改善字幕条即时性。

## 参考

- SenseVoice 预训练模型与下载：https://k2-fsa.github.io/sherpa/onnx/sense-voice/pretrained.html
- sherpa-onnx Rust API 示例：https://github.com/k2-fsa/sherpa-onnx 根目录 rust-api-examples
- crates.io sherpa-onnx：https://crates.io/crates/sherpa-onnx
- Qwen3-ASR int8 模型：https://huggingface.co/csukuangfj2/sherpa-onnx-qwen3-asr-0.6B-int8-2026-03-25
- 流式 Zipformer bilingual：https://k2-fsa.github.io/sherpa-onnx/pretrained_models/online-transducer/zipformer-transducer-models.html
- SenseVoice 上游：https://github.com/FunAudioLLM/SenseVoice

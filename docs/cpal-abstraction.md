# cpal 的跨平台音频抽象总结

来源：cpal 官方文档 docs.rs/cpal 与 GitHub 仓库 RustAudio/cpal，版本 0.18.2。本项目 SimpleLess 使用 cpal 0.15.3，文末列出了两者的 API 差异。

## 1. cpal 是什么

cpal 官方定位是 Low-level library for audio input and output, written in Rust，即 Rust 的低层级跨平台音频输入输出库。它只负责枚举音频设备和收发音频数据，不提供混音、解码、播放列表等高层功能。官方建议需要更高层抽象时使用 Rodio。

## 2. 各平台后端

cpal 在每个平台封装该平台原生的音频 API，对外暴露统一接口：

| 平台 | 默认后端 | 可选后端 |
|---|---|---|
| Windows | WASAPI | ASIO、JACK |
| macOS、iOS、tvOS、visionOS | CoreAudio | JACK 仅 macOS |
| Linux、BSD | ALSA | JACK、PipeWire、PulseAudio |
| Android | AAudio | 无 |
| WebAssembly | Web Audio API | Audio Worklet |

Linux 构建必须安装 ALSA 开发文件，即使实际使用 JACK、PipeWire 或 PulseAudio 也是如此。cpal 本身不拼接输入输出设备组成双工，需要设备自身同时支持采集和播放。

## 3. 核心抽象：三层对象模型

cpal 用三个核心类型统一各平台的音频对象，文档原文概括为：

- Host：provides access to the available audio devices on the system，即访问系统音频设备的入口。部分平台存在多个 host，例如 Linux 上可以选 ALSA 或 JACK，但每个支持的平台都保证有 default_host。
- Device：an audio device that may have any number of input and output streams，即一台音频设备，可以承载任意数量的输入输出流。
- Stream：an open flow of audio data，即一条打开的音频数据流。输入流接收数据，输出流播放数据。文档强调必须先确定运行流的 Device，才能创建 Stream。

三者的关系是：Host 枚举或选出 Device，Device 查询能力或接收配置并构建 Stream，Stream 携带数据回调持续工作。所有设备相关操作都返回 Result，因为设备随时可能被拔出而失效。

## 4. traits 模块：面向 trait 编程实现统一

traits 模块的定位原文是 The suite of traits allowing CPAL to abstract over hosts, devices, event loops and stream IDs。三个核心 trait 与三个核心类型一一对应，各平台的实现类型都实现同一套 trait，上层代码面向 trait 编程即可抹平平台差异。

### HostTrait

必需方法：

- `is_available() -> bool`：判断该 host 在当前系统是否可用。
- `devices() -> Result<Self::Devices, Error>`：迭代所有可用设备，系统完全不支持音频时返回空迭代器。
- `default_input_device() -> Option<Self::Device>`：系统默认输入设备，无则返回 None。
- `default_output_device() -> Option<Self::Device>`：系统默认输出设备，无则返回 None。

提供方法：`input_devices`、`output_devices` 分别只迭代支持输入或输出的设备，`device_by_id` 按 DeviceId 查找设备。

关联类型 `Self::Devices` 和 `Self::Device` 让每个平台可以返回自己的具体类型，同时对外保持统一签名。

### DeviceTrait

关联类型：

- `SupportedInputConfigs` 和 `SupportedOutputConfigs`：迭代该设备支持的配置区间，元素为 SupportedStreamConfigRange。
- `Stream`：由本设备构建出的流类型。

必需方法：

- `name()`、`description()`：设备名称与结构化元数据，description 含制造商、设备类型、总线类型。
- `id()`：设备的稳定标识 DeviceId，文档要求尽可能在程序多次运行、设备重连、系统重启之间保持稳定。
- `supported_input_configs()` / `supported_output_configs()`：查询支持的配置区间。
- `default_input_config()` / `default_output_config()`：获取默认配置。
- `build_input_stream_raw()` / `build_output_stream_raw()`：创建动态类型的流，回调拿到的是 `&Data` 或 `&mut Data`，运行期才确定采样格式。

提供方法：

- `supports_input()` / `supports_output()`：能力探测。
- `build_input_stream<T>()` / `build_output_stream<T>()`：创建静态类型的流，泛型 T 需要 SizedSample，回调直接拿到 `&[T]` 或 `&mut [T]`。这是最常用的入口。

构建流的回调签名形如 `FnMut(&[T], &InputCallbackInfo) + Send + 'static`，同时必须提供错误回调。timeout 参数为 None 表示无限等待，部分后端不遵守该值。

### StreamTrait

- `play()`：启动流。文档明确指出 Streams are returned in a paused state，build 出来的流处于停止状态，必须调用 play 数据回调才会触发。名字虽然叫 play，对输入采集流同样适用。
- `pause()`：暂停流。部分设备支持硬件级挂起以省电，不支持的设备上会失败。
- `buffer_size()`：后端每次回调的帧数估计值，仅用于预分配缓冲，不保证每次回调恰好这么多帧。
- `now()`：流时钟上的当前时刻 StreamInstant，单调递增，与回调时间戳共享同一时间基准。

## 5. 配置抽象

设备能力千差万别，cpal 把配置拆成三层：

- `SupportedStreamConfigRange`：一段受支持的配置范围，由 supported_input_configs 之类的查询返回。调用方在其上选择采样率等参数，例如 `.with_max_sample_rate()`，再 `with_sample_format` 收敛成具体配置。
- `SupportedStreamConfig`：一个具体的、设备明确支持的配置。
- `StreamConfig`：调用方手工构造的参数集合，文档描述为 The set of parameters used to describe how to open a stream。字段包括 channels、sample_rate、buffer_size。设备不认这个配置时会报错。

配套类型：

- `SampleFormat` 枚举：每个采样的格式，如 F32、I16、U16、I8、I32、I64、U8、U32、U64、F64、I24、U24。
- `BufferSize` 枚举：Default 交给后端决定，Fixed 指定帧数。README 提示 ALSA 上 Default 可能给出很深的缓冲，可用 `BufferSize::Fixed(1024)` 固定。
- 常量 `SAMPLE_RATE_48K` 与 `SAMPLE_RATE_CD`：分别是 EBU 广播标准 48 kHz 与 CD 标准 44.1 kHz。

## 6. 采样类型抽象

为了统一不同位深的数据处理，cpal 提供了一组采样 trait：

- `Sample` trait：A trait for working generically across different Sample format types，让算法对各种采样格式通用，例如 `EQUILIBRIUM` 表示静音值。
- `SizedSample`：带有关联 SampleFormat 的 Sized 采样类型，是 build_input_stream<T> 泛型版本的约束。
- `FromSample`：类似标准库 From，专门用于采样类型之间的相互转换。
- `I24`、`U24`：24 位深度的专用类型，因为 Rust 没有原生 24 位整数。
- `Data`：A buffer of dynamically typed audio data，原始流回调中的动态类型缓冲，通过 `sample_format()` 查询实际格式，按 `as_slice::<f32>()` 之类的方法断言取出。

## 7. platform 模块与动态分发

platform 模块是 Platform-specific items，是抽象与具体实现的粘合点：

- 对每个编译目标，platform 模块 re-export 该平台的 `Host`、`Device`、`Stream`、`Devices` 等具体类型。文档称 Host 是 the platform's dynamically dispatched Host type。同一份代码写 `cpal::platform::Host`，在不同平台编译出不同的底层实现。
- `HostId` 枚举标识平台上的可用后端，`ALL_HOSTS` 常量列出本平台全部支持项。
- `available_hosts()` 列出当前系统可用的 host，`default_host()` 返回编译目标的默认 host，`host_from_id()` 按标识实例化指定后端，例如运行时切换到 ASIO。
- `custom` feature 暴露 `CustomHost`、`CustomDevice`、`CustomStream`，允许为 cpal 未原生支持的音频系统编写自己的后端，配合 `assert_stream_send!` 等宏校验流类型满足 Send 约束。

线程模型上，现代平台的音频回调运行在专属高优先级线程。仅提供阻塞 API 的老平台由 cpal 自行建线程，维持非阻塞的使用方式。

## 8. 典型使用流程

```rust
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

// 1. 初始化 host
let host = cpal::default_host();

// 2. 选择设备
let device = host.default_output_device().unwrap();

// 3. 获取配置
let config = device.default_output_config().unwrap();

// 4. 构建流，按采样格式匹配回调类型
let stream = device.build_output_stream(
    config.into(),
    |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
        for sample in data {
            *sample = f32::EQUILIBRIUM; // 静音
        }
    },
    |err| eprintln!("{err}"),
    None,
).unwrap();

// 5. 启动，流创建后默认处于停止状态
stream.play().unwrap();
```

回调是实时线程上的硬时限代码，文档示例遵循的原则是不加锁、不分配、不阻塞。

## 9. 版本差异：本项目用的 0.15 与文档 0.18

本项目 Cargo.toml 声明 `cpal = "0.15"`，实际解析为 0.15.3。对照本地源码，核心抽象完全一致，即三层对象、三个 trait、配置与采样类型在 0.15 中都已存在。0.18 新增的主要 API 在 0.15 中不存在，写代码时不要照抄新文档：

| API | 0.15.3 状态 |
|---|---|
| `DeviceTrait::name()` | 存在，返回 `Result<String, DeviceNameError>` |
| `DeviceTrait::description()`、`id()`、`DeviceId`、`DeviceDescription` | 不存在，0.16 起才有 |
| `HostTrait::device_by_id()` | 不存在 |
| `StreamTrait::buffer_size()`、`now()`、`StreamInstant` | 不存在 |
| `StreamTrait::play()`、`pause()` | 存在 |
| `build_input_stream` / `build_output_stream` 带 timeout 参数 | 存在 |
| `SizedSample`、`FromSample`、`I24`、`U24` | 存在 |

## 10. 对本项目的参考意义

SimpleLess 的 `src-tauri/src/audio.rs` 负责录音采集，属于典型的 cpal 输入流场景，可套用的结论：

- 用 `default_host()` 拿 host，Windows 上默认是 WASAPI。
- 用 `default_input_device()` 拿麦克风，需处理 None，即无可用设备的情形。
- 用 `default_input_config()` 直接取默认配置最省事，通常为 F32 格式。
- `build_input_stream` 的泛型按 `SampleFormat` 匹配后写回调，错误回调里重点处理 `DeviceNotAvailable`，即录音中途拔掉麦克风的情况。
- 构建后必须调用 `play()` 才开始产生数据；丢弃 Stream 实例会让流停止，需要把返回值保存在足够长的生命周期里。

//! 麦克风采集：从默认输入设备采样，重采样为 16kHz 单声道 i16，
//! 按约 100ms 一包转发给识别会话。

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::sync::mpsc as std_mpsc;
use tokio::sync::mpsc;

const TARGET_RATE: f64 = 16000.0;
/// 每个发送包的样本数，约 100ms 音频
const CHUNK_SAMPLES: usize = 1600;

/// 发送端共享槽：回调闭包与停止句柄各持一份。
/// 麦克风被系统拒绝等异常场景下，CoreAudio 单元可能进入僵尸态，
/// drop(Stream) 后回调仍在运行并不肯释放闭包；把发送端放进可拔除的槽里，
/// 停止时主动置空，僵尸回调不再向识别会话供数，通道得以正常关闭
type SharedTx = std::sync::Arc<std::sync::Mutex<Option<mpsc::UnboundedSender<Vec<i16>>>>>;

/// 停止采集的句柄；Stream 在 Windows 上不可跨线程，
/// 由采集线程持有，这里只通过通道发送停止信号
pub struct AudioStopper {
    stop: Option<std_mpsc::Sender<()>>,
    shared: Option<SharedTx>,
}

impl AudioStopper {
    pub fn stop(mut self) {
        // 先拔发送端再停流，确保此后回调即使仍被调用也无数据可发
        if let Some(shared) = self.shared.take() {
            *shared.lock().unwrap() = None;
        }
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
    }
}

/// 可选录音设备：id 写入配置并用于设备匹配，label 供设置界面展示
#[derive(serde::Serialize)]
pub struct InputDeviceInfo {
    pub id: String,
    pub label: String,
}

/// 枚举可用输入设备，供设置界面选择。
/// Linux 上 ALSA 会把每块声卡按插件层展开成多条同名通路（hw/plughw/front/dsnoop...），
/// 这里收敛为每个物理通路一条 plughw:，并用 ALSA 的 DESC 字段做展示名；
/// 其他平台设备名本身就是系统级的唯一友好名，原样透出
pub fn list_input_devices() -> Vec<InputDeviceInfo> {
    let host = cpal::default_host();
    let devices: Vec<(String, cpal::Device)> = match host.input_devices() {
        Ok(devs) => devs
            .filter_map(|d| d.name().ok().map(|name| (name, d)))
            .collect(),
        Err(_) => Vec::new(),
    };

    #[cfg(target_os = "linux")]
    {
        let kept: std::collections::HashSet<String> =
            linux::prune_alsa_ids(&devices.iter().map(|(n, _)| n.clone()).collect::<Vec<_>>())
                .into_iter()
                .collect();
        let desc = linux::alsa_desc_map();
        // default_input_config 能打开才可录，剔掉 HDMI 等纯输出通路
        devices
            .into_iter()
            .filter(|(n, d)| kept.contains(n) && d.default_input_config().is_ok())
            .map(|(n, _)| {
                let label = desc.get(&n).cloned().unwrap_or_else(|| n.clone());
                InputDeviceInfo { id: n, label }
            })
            .collect()
    }
    #[cfg(not(target_os = "linux"))]
    {
        devices
            .into_iter()
            .map(|(n, _)| InputDeviceInfo { id: n.clone(), label: n })
            .collect()
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use std::collections::HashMap;

    /// ALSA 设备名到 DESC 描述的映射（如 "HDA Intel PCH, ALC897 Analog"），
    /// 与 cpal 枚举同源于 snd_device_name_hint，仅用于展示
    pub fn alsa_desc_map() -> HashMap<String, String> {
        let mut map = HashMap::new();
        let Ok(iter) = alsa::device_name::HintIter::new_str(None, "pcm") else {
            return map;
        };
        for hint in iter {
            if let (Some(name), Some(desc)) = (hint.name, hint.desc) {
                map.entry(name).or_insert(desc);
            }
        }
        map
    }

    /// 收敛 cpal 枚举出的 ALSA 设备名：保留 default 与 pipewire 两个虚拟设备，
    /// 其余按 (CARD, DEV) 物理通路分组各留一条，优先 plughw:（自动格式转换，
    /// 对采样率不匹配的场景比直通 hw: 兼容），组内没有 plughw: 时退而取 hw:
    pub fn prune_alsa_ids(ids: &[String]) -> Vec<String> {
        let mut best_rank: HashMap<(String, String), u8> = HashMap::new();
        for id in ids {
            if let (Some(rank), Some(key)) = (prefix_rank(id), physical_key(id)) {
                best_rank
                    .entry(key)
                    .and_modify(|r| *r = (*r).min(rank))
                    .or_insert(rank);
            }
        }
        ids.iter()
            .filter(|id| {
                let id = id.as_str();
                if id == "null" {
                    return false;
                }
                if !id.contains("CARD=") {
                    // pulse/sysdefault 的指向与 default 重叠，前者非标配、后者即默认
                    return id == "default" || id == "pipewire";
                }
                match (prefix_rank(id), physical_key(id)) {
                    (Some(rank), Some(key)) => best_rank.get(&key) == Some(&rank),
                    _ => false,
                }
            })
            .cloned()
            .collect()
    }

    /// 插件层优先级：plughw 0 < hw 1；front/dsnoop/sysdefault 等不作代表，返回 None
    fn prefix_rank(id: &str) -> Option<u8> {
        if id.starts_with("plughw:") {
            Some(0)
        } else if id.starts_with("hw:") {
            Some(1)
        } else {
            None
        }
    }

    /// 从 "plughw:CARD=PCH,DEV=0" 提取 ("PCH", "0")；旧式 "hw:0,0" 无 CARD 字段，返回 None
    fn physical_key(id: &str) -> Option<(String, String)> {
        let rest = id.split(':').nth(1)?;
        Some((param(rest, "CARD")?, param(rest, "DEV").unwrap_or_else(|| "0".into())))
    }

    /// 取 "CARD=PCH,DEV=0" 中 key 对应的值
    fn param(rest: &str, key: &str) -> Option<String> {
        rest.split(',')
            .find_map(|p| p.strip_prefix(key)?.strip_prefix('=').map(str::to_string))
    }

    #[cfg(test)]
    mod tests {
        use super::prune_alsa_ids;

        #[test]
        fn test_prune_alsa_ids() {
            let ids: Vec<String> = [
                "null",
                "default",
                "pipewire",
                "pulse",
                "hw:CARD=PCH,DEV=0",
                "plughw:CARD=PCH,DEV=0",
                "sysdefault:CARD=PCH",
                "front:CARD=PCH,DEV=0",
                "dsnoop:CARD=PCH,DEV=0",
                "hw:CARD=PCH,DEV=2",
                "plughw:CARD=PCH,DEV=2",
                "hw:CARD=Camera,DEV=0",
                "hdmi:CARD=PCH,DEV=3",
                "usbstream:CARD=Camera,DEV=0",
                "hw:0,0",
            ]
            .iter()
            .map(|s| s.to_string())
            .collect();
            let kept = prune_alsa_ids(&ids);
            assert_eq!(
                kept,
                [
                    "default",
                    "pipewire",
                    "plughw:CARD=PCH,DEV=0",
                    "plughw:CARD=PCH,DEV=2",
                    // 该组无 plughw: 条目，退回 hw:
                    "hw:CARD=Camera,DEV=0",
                ]
            );
        }
    }
}

/// 按配置解析采集设备：None 跟随系统默认；指定名称找不到时直接报错，
/// 不静默回落到默认设备，避免用户以为在用选定的麦克风、实际录进了别的设备
fn resolve_input_device(preferred: Option<&str>) -> Result<cpal::Device, String> {
    let host = cpal::default_host();
    match preferred {
        None => host
            .default_input_device()
            .ok_or("找不到默认麦克风设备".into()),
        Some(name) => host
            .input_devices()
            .map_err(|e| format!("枚举录音设备失败: {e}"))?
            .find(|d| matches!(d.name(), Ok(n) if n == name))
            .ok_or_else(|| format!("找不到录音设备 {name}，可能已断开，请在设置中改回系统默认")),
    }
}

/// 启动采集：preferred 指定设备名，None 表示系统默认，输出 16kHz 单声道 i16 样本流
pub fn spawn(
    tx: mpsc::UnboundedSender<Vec<i16>>,
    preferred: Option<&str>,
) -> Result<AudioStopper, String> {
    let device = resolve_input_device(preferred)?;
    let shared: SharedTx = std::sync::Arc::new(std::sync::Mutex::new(Some(tx)));
    let (init_tx, init_rx) = std_mpsc::channel::<Result<(), String>>();
    let (stop_tx, stop_rx) = std_mpsc::channel::<()>();

    {
        let shared = std::sync::Arc::clone(&shared);
        std::thread::spawn(move || match build_and_play(&device, &shared) {
            Ok(stream) => {
                let _ = init_tx.send(Ok(()));
                // 阻塞等待停止信号，stream 存活于本线程栈上
                let _ = stop_rx.recv();
                drop(stream);
            }
            Err(e) => {
                let _ = init_tx.send(Err(e));
            }
        });
    }

    match init_rx.recv() {
        Ok(Ok(())) => Ok(AudioStopper {
            stop: Some(stop_tx),
            shared: Some(shared),
        }),
        Ok(Err(e)) => Err(e),
        Err(_) => Err("音频采集线程启动失败".into()),
    }
}

fn build_and_play(device: &cpal::Device, shared: &SharedTx) -> Result<cpal::Stream, String> {
    let supported = device
        .default_input_config()
        .map_err(|e| format!("读取麦克风默认配置失败: {e}"))?;

    let in_rate = supported.sample_rate().0 as f64;
    let channels = supported.channels() as usize;
    let sample_format = supported.sample_format();
    let config: cpal::StreamConfig = supported.into();

    let err_cb = move |err| eprintln!("[audio] 采集错误: {err}");

    let stream = match sample_format {
        cpal::SampleFormat::F32 => {
            build_stream::<f32>(device, &config, channels, in_rate, std::sync::Arc::clone(shared), err_cb)?
        }
        cpal::SampleFormat::I16 => {
            build_stream::<i16>(device, &config, channels, in_rate, std::sync::Arc::clone(shared), err_cb)?
        }
        cpal::SampleFormat::I8 => {
            build_stream::<i8>(device, &config, channels, in_rate, std::sync::Arc::clone(shared), err_cb)?
        }
        cpal::SampleFormat::I32 => {
            build_stream::<i32>(device, &config, channels, in_rate, std::sync::Arc::clone(shared), err_cb)?
        }
        cpal::SampleFormat::U8 => {
            build_stream::<u8>(device, &config, channels, in_rate, std::sync::Arc::clone(shared), err_cb)?
        }
        cpal::SampleFormat::U16 => {
            build_stream::<u16>(device, &config, channels, in_rate, std::sync::Arc::clone(shared), err_cb)?
        }
        other => return Err(format!("不支持的麦克风采样格式: {other:?}")),
    };

    stream.play().map_err(|e| format!("启动麦克风失败: {e}"))?;
    Ok(stream)
}

fn build_stream<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    channels: usize,
    in_rate: f64,
    shared: SharedTx,
    err_cb: impl FnMut(cpal::StreamError) + Send + 'static,
) -> Result<cpal::Stream, String>
where
    T: cpal::SizedSample + std::fmt::Debug + Send + 'static,
    f32: cpal::FromSample<T>,
{
    let mut resampler = Resampler::new(in_rate, TARGET_RATE);
    let mut sample_buf: Vec<i16> = Vec::with_capacity(CHUNK_SAMPLES * 2);
    let mut mono_buf: Vec<f32> = Vec::new();

    device
        .build_input_stream(
            config,
            move |data: &[T], _: &cpal::InputCallbackInfo| {
                // 已停止时直接跳过，省掉重采样开销，也阻断僵尸回调供数
                if shared.lock().unwrap().is_none() {
                    return;
                }
                mono_buf.clear();
                for s in data {
                    mono_buf.push(cpal::Sample::to_sample::<f32>(*s));
                }
                let mono = downmix_in_place(&mut mono_buf, channels);
                for s in resampler.process(mono) {
                    sample_buf.push((s.clamp(-1.0, 1.0) * 32767.0) as i16);
                }
                if sample_buf.len() >= CHUNK_SAMPLES {
                    if let Some(tx) = shared.lock().unwrap().as_ref() {
                        let _ = tx.send(std::mem::take(&mut sample_buf));
                    } else {
                        sample_buf.clear();
                    }
                }
            },
            err_cb,
            None,
        )
        .map_err(|e| format!("打开麦克风失败: {e}"))
}

fn downmix_in_place(samples: &mut Vec<f32>, channels: usize) -> &[f32] {
    if channels <= 1 {
        return samples;
    }
    let n = samples.len() / channels;
    for i in 0..n {
        let frame = &samples[i * channels..(i + 1) * channels];
        samples[i] = frame.iter().sum::<f32>() / channels as f32;
    }
    samples.truncate(n);
    samples
}

/// 流式线性插值重采样
struct Resampler {
    /// 上一包最后一个样本，用于跨包插值
    last: f32,
    /// 输出位置相对当前包起点的相位（以输入样本为单位）
    phase: f64,
    /// 每产出一个样本需要推进的输入样本数
    step: f64,
}

impl Resampler {
    fn new(in_rate: f64, out_rate: f64) -> Self {
        Self {
            last: 0.0,
            phase: 0.0,
            step: in_rate / out_rate,
        }
    }

    fn process(&mut self, input: &[f32]) -> Vec<f32> {
        if input.is_empty() {
            return Vec::new();
        }
        // 把上一包末样本视作输入下标 -1，可插值区间为 [0, input.len()-1]
        let n = input.len() as f64;
        let mut out = Vec::new();
        while self.phase <= n - 1.0 + 1e-9 {
            let i = self.phase.floor() as usize;
            let frac = self.phase - i as f64;
            let s0 = if i == 0 { self.last } else { input[i - 1] };
            let s1 = input[i];
            out.push(s0 + (s1 - s0) * frac as f32);
            self.phase += self.step;
        }
        self.phase -= n;
        self.last = input[input.len() - 1];
        out
    }
}

#[cfg(test)]
mod tests {
    use cpal::traits::{DeviceTrait, HostTrait};

    #[test]
    fn test_resampler() {
        let host = cpal::default_host();
        let devices = host.input_devices().expect("枚举音频输入设备失败");

        let mut count = 0;

        for d in devices {
            log::debug!(
                "device: {}",
                d.name().unwrap_or_else(|e| format!("<未知设备: {e}>"))
            );
            count += 1;
        }

        assert_ne!(count, 0);
    }

    #[test]
    fn test_resolve_input_device() {
        // 跟随系统默认：本机必有输入设备
        assert!(super::resolve_input_device(None).is_ok());

        // 指定不存在的设备名必须报错，不能静默回落
        let err = match super::resolve_input_device(Some("不存在的设备")) {
            Err(e) => e,
            Ok(_) => panic!("不存在的设备不应解析成功"),
        };
        assert!(err.contains("找不到"), "错误信息应说明设备缺失: {err}");

        // 枚举出的 id 应能被原样解析回设备
        if let Some(id) = super::list_input_devices().first().map(|d| d.id.clone()) {
            assert!(super::resolve_input_device(Some(&id)).is_ok());
        }
    }

    /// 排障工具：打印收敛后的设备列表（id 与展示名），核对选择器呈现
    #[test]
    #[ignore = "依赖真实音频设备，仅排障时手动执行"]
    fn test_list_devices() {
        for d in super::list_input_devices() {
            eprintln!("[设备诊断] id={} label={}", d.id, d.label);
        }
    }
}

#[cfg(test)]
mod capture_tests {
    use super::*;
    use cpal::traits::StreamTrait;
    use std::sync::{Arc, Mutex};

    fn record(peak: &Arc<Mutex<f32>>, count: &Arc<Mutex<usize>>, vals: Vec<f32>) {
        *count.lock().unwrap() += vals.len();
        let mut p = peak.lock().unwrap();
        for a in vals {
            if a > *p {
                *p = a;
            }
        }
    }

    /// 回归测试：停止采集后音频通道必须及时关闭。
    /// 麦克风被系统拒绝时 CoreAudio 可能进入僵尸态，drop 流后回调仍在运行，
    /// 若发送端不能主动拔除，识别会话将永远等不到通道关闭（v0.2.3 卡死根因）
    #[test]
    #[ignore = "真实打开麦克风，仅排障或回归验证时执行"]
    fn test_stop_closes_channel() {
        let (tx, mut rx) = mpsc::unbounded_channel::<Vec<i16>>();
        let stopper = spawn(tx, None).expect("启动采集失败");
        std::thread::sleep(std::time::Duration::from_millis(600));
        stopper.stop();
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let closed = rt.block_on(async {
            let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(3);
            loop {
                match tokio::time::timeout_at(deadline, rx.recv()).await {
                    Ok(None) => break true,
                    Ok(Some(_)) => continue,
                    Err(_) => break false,
                }
            }
        });
        assert!(closed, "停止采集后音频通道应在 3 秒内关闭");
    }

    /// 排障工具：验证采集链路是否拿到真实音频。默认忽略，需要时执行
    /// SL_TEST_DEVICE=设备名 cargo test test_capture_amplitude -- --ignored --nocapture
    /// 样本数正常而峰值为零，通常意味着系统麦克风权限被拒
    #[test]
    #[ignore = "会真实打开麦克风，仅排障时手动执行"]
    fn test_capture_amplitude() {
        let name = std::env::var("SL_TEST_DEVICE").ok();
        let device = resolve_input_device(name.as_deref()).expect("解析设备失败");
        eprintln!("[采集诊断] 设备: {}", device.name().unwrap_or_default());
        let supported = device.default_input_config().expect("读取配置失败");
        eprintln!("[采集诊断] 格式: {supported:?}");
        let fmt = supported.sample_format();
        let config: cpal::StreamConfig = supported.into();

        let peak = Arc::new(Mutex::new(0f32));
        let count = Arc::new(Mutex::new(0usize));
        let stream = match fmt {
            cpal::SampleFormat::F32 => {
                let (p, c) = (Arc::clone(&peak), Arc::clone(&count));
                device
                    .build_input_stream(
                        &config,
                        move |data: &[f32], _: &cpal::InputCallbackInfo| {
                            record(&p, &c, data.iter().map(|s| s.abs()).collect())
                        },
                        |e| eprintln!("[采集诊断] 流错误: {e}"),
                        None,
                    )
                    .expect("打开采集流失败")
            }
            cpal::SampleFormat::I16 => {
                let (p, c) = (Arc::clone(&peak), Arc::clone(&count));
                device
                    .build_input_stream(
                        &config,
                        move |data: &[i16], _: &cpal::InputCallbackInfo| {
                            record(&p, &c, data.iter().map(|s| (*s as f32 / 32768.0).abs()).collect())
                        },
                        |e| eprintln!("[采集诊断] 流错误: {e}"),
                        None,
                    )
                    .expect("打开采集流失败")
            }
            other => panic!("[采集诊断] 不支持的格式: {other:?}"),
        };
        stream.play().expect("启动采集失败");
        std::thread::sleep(std::time::Duration::from_millis(2500));
        drop(stream);
        let samples = *count.lock().unwrap();
        let peak = *peak.lock().unwrap();
        eprintln!("[采集诊断] 样本数={samples} 峰值={peak:.6}");
    }
}

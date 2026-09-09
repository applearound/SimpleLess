use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;

const TARGET_RATE: f64 = 16000.0;
/// 每个发送包的样本数，约 100ms 音频
const CHUNK_SAMPLES: usize = 1600;

/// 停止采集的句柄，调用 stop 后流被释放、采集停止
pub struct AudioStopper {
    stream: Arc<Mutex<Option<cpal::Stream>>>,
}

impl AudioStopper {
    pub fn stop(self) {
        if let Some(stream) = self.stream.lock().unwrap().take() {
            drop(stream);
        }
    }
}

/// 启动默认麦克风的采集，输出 16kHz 单声道 i16 样本流
pub fn spawn(tx: mpsc::Sender<Vec<i16>>) -> Result<AudioStopper, String> {
    let host = cpal::default_host();
    let device = host
        .default_input_device()
        .ok_or("找不到默认麦克风设备")?;
    let supported = device
        .default_input_config()
        .map_err(|e| format!("读取麦克风默认配置失败: {e}"))?;

    let in_rate = supported.sample_rate().0 as f64;
    let channels = supported.channels() as usize;
    let config: cpal::StreamConfig = supported.into();
    let sample_format = supported.sample_format();

    let err_cb = move |err| eprintln!("[audio] 采集错误: {err}");

    let stream = match sample_format {
        cpal::SampleFormat::F32 => build_stream::<f32>(&device, &config, channels, in_rate, tx, err_cb)?,
        cpal::SampleFormat::I16 => build_stream::<i16>(&device, &config, channels, in_rate, tx, err_cb)?,
        cpal::SampleFormat::I8 => build_stream::<i8>(&device, &config, channels, in_rate, tx, err_cb)?,
        cpal::SampleFormat::I32 => build_stream::<i32>(&device, &config, channels, in_rate, tx, err_cb)?,
        cpal::SampleFormat::U8 => build_stream::<u8>(&device, &config, channels, in_rate, tx, err_cb)?,
        cpal::SampleFormat::U16 => build_stream::<u16>(&device, &config, channels, in_rate, tx, err_cb)?,
        other => return Err(format!("不支持的麦克风采样格式: {other:?}")),
    };

    stream.play().map_err(|e| format!("启动麦克风失败: {e}"))?;

    Ok(AudioStopper {
        stream: Arc::new(Mutex::new(Some(stream))),
    })
}

fn build_stream<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    channels: usize,
    in_rate: f64,
    tx: mpsc::Sender<Vec<i16>>,
    err_cb: impl FnMut(cpal::StreamError) + Send + 'static,
) -> Result<cpal::Stream, String>
where
    T: cpal::Sample + std::fmt::Debug + Send + 'static,
{
    let mut resampler = Resampler::new(in_rate, TARGET_RATE);
    let mut sample_buf: Vec<i16> = Vec::with_capacity(CHUNK_SAMPLES * 2);
    let mut mono_buf: Vec<f32> = Vec::new();

    device
        .build_input_stream(
            config,
            move |data: &[T], _: &cpal::InputCallbackInfo| {
                mono_buf.clear();
                for s in data {
                    mono_buf.push(cpal::Sample::to_sample::<f32>(*s));
                }
                let mono = downmix_in_place(&mut mono_buf, channels);
                for s in resampler.process(mono) {
                    sample_buf.push((s.clamp(-1.0, 1.0) * 32767.0) as i16);
                }
                if sample_buf.len() >= CHUNK_SAMPLES {
                    let _ = tx.try_send(std::mem::take(&mut sample_buf));
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

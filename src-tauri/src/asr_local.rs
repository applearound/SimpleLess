//! 本地语音识别：sherpa-onnx 运行 SenseVoice 离线模型，Silero VAD 按语音段
//! 切分逐段识别，对外复用云端会话的 AsrSession 三通道接口，调用方无感切换。

use crate::asr::AsrSession;
use sherpa_onnx::{
    OfflineRecognizer, OfflineRecognizerConfig, OfflineSenseVoiceModelConfig, VadModelConfig,
    VoiceActivityDetector,
};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, oneshot, watch};

/// 模型目录内的固定文件名，下载器与就绪探测共用
pub const MODEL_FILE: &str = "model.int8.onnx";
pub const TOKENS_FILE: &str = "tokens.txt";
pub const VAD_FILE: &str = "silero_vad.onnx";

/// 模型目录是否已具备全部文件
pub fn model_ready(model_dir: &Path) -> bool {
    [MODEL_FILE, TOKENS_FILE, VAD_FILE]
        .iter()
        .all(|f| model_dir.join(f).is_file())
}

/// 启动一次本地识别会话。文件缺失立即报错，模型加载耗时约一秒，
/// 放在后台线程进行，失败经 done 通道返回，不阻塞异步运行时。
pub fn start(model_dir: &Path) -> Result<AsrSession, String> {
    if !model_ready(model_dir) {
        return Err("本地识别模型不完整，请在设置中重新下载".into());
    }

    let (audio_tx, audio_rx) = mpsc::unbounded_channel::<Vec<i16>>();
    let (partial_tx, partial_rx) = watch::channel(String::new());
    let (done_tx, done_rx) = oneshot::channel();

    let dir = model_dir.to_path_buf();
    std::thread::spawn(move || {
        if let Err(e) = run(&dir, audio_rx, partial_tx, done_tx) {
            eprintln!("[asr_local] 会话失败: {e}");
        }
    });

    Ok(AsrSession {
        audio_tx,
        partial_rx,
        done_rx,
    })
}

/// 后台线程主体：加载模型后进入交换循环，音频端关闭后收尾返回全文
fn run(
    model_dir: &PathBuf,
    mut audio_rx: mpsc::UnboundedReceiver<Vec<i16>>,
    partial_tx: watch::Sender<String>,
    done_tx: oneshot::Sender<Result<String, String>>,
) -> Result<(), String> {
    let recognizer = create_recognizer(model_dir)?;
    let vad = create_vad(model_dir)?;

    const SAMPLE_RATE: i32 = 16000;
    /// Silero VAD 的固定窗口大小，示例明确要求不要改动
    const WINDOW_SIZE: usize = 512;
    /// 说话中当前句预览识别的最小间隔
    const INTERIM_INTERVAL: Duration = Duration::from_millis(250);

    let mut buffer: Vec<f32> = Vec::new();
    let mut offset: usize = 0;
    let mut finalized = String::new();
    let mut speech_started = false;
    let mut last_decode = Instant::now();

    while let Some(samples) = audio_rx.blocking_recv() {
        buffer.extend(samples.iter().map(|&s| s as f32 / 32768.0));

        while offset + WINDOW_SIZE <= buffer.len() {
            vad.accept_waveform(&buffer[offset..offset + WINDOW_SIZE]);
            if !speech_started && vad.detected() {
                speech_started = true;
            }
            offset += WINDOW_SIZE;
        }

        // 未开始说话时限制静音缓冲，避免长按不放占用内存
        if !speech_started && buffer.len() > 10 * WINDOW_SIZE {
            let keep_from = buffer.len() - 10 * WINDOW_SIZE;
            buffer.drain(..keep_from);
            offset = offset.saturating_sub(keep_from);
        }

        // 当前句预览：对未定稿缓冲整体识别，拼接已定稿文本上屏
        if speech_started && last_decode.elapsed() >= INTERIM_INTERVAL {
            let interim = decode(&recognizer, SAMPLE_RATE, &buffer);
            if !interim.is_empty() {
                let _ = partial_tx.send(format!("{finalized}{interim}"));
            }
            last_decode = Instant::now();
        }

        // VAD 定稿段：逐段识别并追加到全文
        while let Some(segment) = vad.front() {
            let text = decode(&recognizer, SAMPLE_RATE, segment.samples());
            vad.pop();
            if !text.is_empty() {
                finalized.push_str(&text);
                let _ = partial_tx.send(finalized.clone());
            }
            buffer.clear();
            offset = 0;
            speech_started = false;
        }
    }

    // 收尾：音频端已全部关闭，VAD 内残留段与缓冲尾段都要识别完
    while let Some(segment) = vad.front() {
        let text = decode(&recognizer, SAMPLE_RATE, segment.samples());
        vad.pop();
        if !text.is_empty() {
            finalized.push_str(&text);
        }
        buffer.clear();
    }
    if speech_started && !buffer.is_empty() {
        let text = decode(&recognizer, SAMPLE_RATE, &buffer);
        finalized.push_str(&text);
    }
    let _ = partial_tx.send(finalized.clone());
    let _ = done_tx.send(Ok(finalized));

    Ok(())
}

fn decode(recognizer: &OfflineRecognizer, sample_rate: i32, samples: &[f32]) -> String {
    let stream = recognizer.create_stream();
    stream.accept_waveform(sample_rate, samples);
    recognizer.decode(&stream);
    stream.get_result().map(|r| r.text).unwrap_or_default()
}

fn create_recognizer(model_dir: &Path) -> Result<OfflineRecognizer, String> {
    let mut config = OfflineRecognizerConfig::default();
    config.model_config.sense_voice = OfflineSenseVoiceModelConfig {
        model: Some(model_dir.join(MODEL_FILE).to_string_lossy().into_owned()),
        language: Some("auto".into()),
        use_itn: true,
    };
    config.model_config.tokens =
        Some(model_dir.join(TOKENS_FILE).to_string_lossy().into_owned());
    config.model_config.num_threads = 2;
    OfflineRecognizer::create(&config).ok_or("加载本地识别模型失败，文件可能已损坏，请在设置中重新下载".to_string())
}

fn create_vad(model_dir: &Path) -> Result<VoiceActivityDetector, String> {
    let mut config = VadModelConfig::default();
    config.silero_vad.model = Some(model_dir.join(VAD_FILE).to_string_lossy().into_owned());
    config.silero_vad.threshold = 0.5;
    config.silero_vad.min_silence_duration = 0.25;
    config.silero_vad.min_speech_duration = 0.25;
    config.silero_vad.max_speech_duration = 8.0;
    config.silero_vad.window_size = 512;
    config.sample_rate = 16000;
    VoiceActivityDetector::create(&config, 20.0)
        .ok_or("加载语音活动检测失败，模型文件可能已损坏，请在设置中重新下载".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 模型目录位置：优先环境变量 SIMPLELESS_LOCAL_MODEL_DIR，
    /// 否则用开发机上的固定路径，便于手工验证本地识别全流程。
    /// cargo test test_local_asr -- --ignored --nocapture
    #[test]
    #[ignore = "需要本地模型文件，仅本地验证时执行"]
    fn test_local_asr_with_wav() {
        let dir = std::env::var("SIMPLELESS_LOCAL_MODEL_DIR")
            .unwrap_or_else(|_| "D:/Dev/models/sense-voice".into());
        let wav = std::env::var("SIMPLELESS_LOCAL_TEST_WAV")
            .unwrap_or_else(|_| "D:/Dev/models/sense-voice/zh.wav".into());
        assert!(model_ready(Path::new(&dir)), "模型文件不完整: {dir}");

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async move {
            let session = start(Path::new(&dir)).expect("启动本地会话失败");
            let wave = sherpa_onnx::Wave::read(&wav).expect("读取测试音频失败");
            let samples_i16: Vec<i16> = wave
                .samples()
                .iter()
                .map(|&s| (s * 32767.0) as i16)
                .collect();
            // 按每次 100 毫秒分块送入，模拟实时麦克风流
            for chunk in samples_i16.chunks(1600) {
                session.audio_tx.send(chunk.to_vec()).expect("发送失败");
                tokio::time::sleep(Duration::from_millis(40)).await;
            }
            drop(session.audio_tx);
            match tokio::time::timeout(Duration::from_secs(20), session.done_rx).await {
                Ok(Ok(Ok(text))) => {
                    assert!(!text.trim().is_empty(), "识别结果为空");
                    eprintln!("[本地识别] 结果: {text}");
                    eprintln!("[本地识别] 最终 partial: {}", session.partial_rx.borrow().clone());
                }
                other => panic!("本地识别会话异常: {other:?}"),
            }
        });
    }
}

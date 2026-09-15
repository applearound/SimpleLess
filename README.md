# SimpleLess

语音优先（Speech First）的桌面输入工具：全程无界面操作，按热键说话，文本插入光标处；改配置也用语音完成。

对标 TypeLess / OpenLess，主打简单易用，初期绑定阿里云百炼平台。

## 使用

1. `bun install`
2. `bun run tauri dev`（开发）或 `bun run tauri build`（打包）
3. 首次启动在设置窗口填入百炼 API Key，验证通过后软件退到托盘，界面不再出现
4. 热键（可在配置文件中改）：
   - `Caps Lock` 听写：按一下开始，再按一下结束；文本按当前档位处理后插入光标处
   - `Ctrl+Caps Lock` 命令：按一下开始，再按一下结束；用语音修改设置，如“切换成原文”“切换成润色”
   - 热键被软件接管后，键盘原有的 Caps Lock 大小写切换功能不再生效，介意可改配置文件换键
5. 左键点托盘图标可重新打开设置窗口

## 架构

- 前端 Vue 3 仅两块屏：首启向导（唯一带输入框的界面）与透明置顶字幕条（纯状态展示，无控件）
- 全部业务逻辑在 Rust 侧：cpal 采集音频并重采样为 16kHz 单声道，tokio-tungstenite 直连百炼 fun-asr-realtime 流式识别，reqwest 调 qwen3.8（OpenAI 兼容接口）做润色与命令路由（function calling），arboard 加 enigo 以剪贴板粘贴方式插入文本
- API Key 存放于系统凭据管理器（keyring），配置存 `config.json`

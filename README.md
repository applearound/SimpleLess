# SimpleLess

简单的语音优先的桌面输入工具：按热键说话，文本插入光标处。

简单易用，现绑定阿里云百炼平台。

## 使用

1. `bun install`
2. `bun run tauri dev`（开发）或 `bun run tauri build`（打包）
3. 首次启动在设置窗口填入百炼 API Key，验证通过后软件退到托盘，界面不再出现
4. 热键（可在配置文件中改）：
   - `Caps Lock` 听写：按一下开始，再按一下结束；文本按当前档位处理后插入光标处
   - `Ctrl+Caps Lock` 命令：按一下开始，再按一下结束；用语音修改设置，如“切换成原文”“切换成润色”
   - 热键被软件接管后，键盘原有的 Caps Lock 大小写切换功能不再生效，介意可改配置文件换键
5. 左键点托盘图标可重新打开设置窗口

## 配置文件

配置存于系统用户目录下的 `com.yezhou.simpleless/config.json`：

- Windows：`%APPDATA%\com.yezhou.simpleless\config.json`
- macOS：`~/Library/Application Support/com.yezhou.simpleless/config.json`
- Linux：`~/.config/com.yezhou.simpleless/config.json`

百炼 API Key 不在此文件中，存于系统凭据管理器，删除或重置 config.json 不会丢失 API Key。

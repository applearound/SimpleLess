# AGENTS.md

## 项目概览

简单的桌面输入工具：按热键说话，文本按档位处理后插入光标处。Tauri 2 架构，前端 Vue 3 + TypeScript + Vite。

## 命令

- 依赖安装：bun install
- 启动应用：bun run tauri dev
- 本地验证构建（无分发包）：bun run build:quick
- 本地验证构建（带分发包，跨平台统一入口在 scripts/build.mjs）：bun run build:dist
- 清理构建产物（dist 与 src-tauri/target）：bun run clean
- 测试：bun run test:rust

## 版本管理

仓库根目录存在 `.jj` 目录时：使用 jujutsu vcs 做版本管理；否则按普通 git 仓库处理。

## 目录

- src/：前端 Vue 界面，仅首启向导与字幕条两屏；样式为 Tailwind CSS v4，UI 组件走 shadcn-vue 风格（src/components/ui/，配置见 components.json）。
- src-tauri/src/：Rust 业务逻辑，音频采集、语音识别、润色与命令路由、文本插入、凭据、配置。
- docs/：调研笔记。

其余细节按任务需要渐进式查阅源码即可。

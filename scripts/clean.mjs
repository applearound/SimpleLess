// 清理可再生的构建产物，用法: bun run clean
import { rmSync } from "node:fs";
import { resolve } from "node:path";

const targets = [
  ["dist", "前端构建产物"],
  ["src-tauri/target", "Rust 构建产物"],
];

for (const [dir, label] of targets) {
  rmSync(resolve(dir), { recursive: true, force: true });
  console.log(`已清理 ${label}: ${dir}`);
}

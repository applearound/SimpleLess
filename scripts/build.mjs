// 跨平台统一构建入口，不带参数时默认 quick
import { spawnSync } from "node:child_process";

const usage = "用法: bun run build:quick 或 bun run build:dist，不带参数默认 quick";
const mode = process.argv[2] ?? "quick";
if (!["quick", "dist"].includes(mode)) {
  console.error(usage);
  process.exit(1);
}

// quick: 只要能直接运行的产物；dist: 当前平台全部安装器
const argsByMode = {
  quick: {
    darwin: ["--bundles", "app"],
    win32: ["--no-bundle"],
    linux: ["--no-bundle"],
  },
  dist: {
    darwin: [],
    win32: [],
    linux: [],
  },
};

const args = argsByMode[mode][process.platform];
if (!args) {
  console.error(`不支持的平台: ${process.platform}\n${usage}`);
  process.exit(1);
}

const result = spawnSync("tauri", ["build", ...args], {
  stdio: "inherit",
  shell: process.platform === "win32",
});
if (result.error) {
  console.error(result.error.message);
}
process.exit(result.status ?? 1);

# AGENTS.md

## 版本管理：本仓库由 jj 管理

本仓库采用 jj（Jujutsu VCS）与 git 共存的 colocate 模式进行本地版本管理。远程仓库仍是普通 git 仓库（GitHub），协作者继续使用 git 不受影响。

### 检测规则

- 仓库根目录存在 `.jj` 目录：本地操作一律使用 jj 命令，遵循下述规则。
- 不存在 `.jj` 目录：按普通 git 仓库处理。

### 使用 jj 时的规则

1. 本地写操作只用 jj。提交、修改描述、拆分、合并、变基、改写历史等操作一律通过 jj 完成。
2. 禁止 git 写操作。不要执行 `git commit`、`git rebase`、`git reset`、`git merge`、`git checkout` 等会移动引用或改写工作区的命令，避免与 jj 的状态管理冲突。
3. git 只读命令不受限。`git log`、`git diff`、`git show` 等查看类命令可正常使用。
4. 远程交互用 jj。拉取用 `jj git fetch`，推送具名书签用 `jj git push`，为当前提交创建远程分支并推送用 `jj git push -c @`。
5. 工作副本即提交。文件保存即自动进入当前提交 `@`，推送前用 `jj log` 和 `jj st` 确认内容，并用 `jj describe -m` 写清提交描述。
6. 推送前先拉取。先 `jj git fetch` 同步远程，再解决可能的冲突；包含冲突的提交会被拒绝推送。
7. 后悔操作用 `jj undo` 撤销上一步，`jj op log` 查看全部操作历史。

### 常用命令对照

| git 习惯 | jj 命令 |
| --- | --- |
| `git status` | `jj st` |
| `git add` + `git commit` | 自动快照，无需 add；`jj describe -m` 写描述，`jj new` 开新提交 |
| `git commit --amend` | 直接修改文件，`@` 自动更新 |
| `git push` | `jj git push` |
| `git pull --rebase` | `jj git fetch` |
| `git log` | `jj log` |
| `git checkout -b` / 新分支 | `jj new -n 分支名` 或 `jj bookmark create 分支名` |
| `git restore <file>` | `jj restore <file>` |

### 其他说明

- `.jj` 目录仅存在于本地，不提交到远程，已列入 `.gitignore`。
- jj 不读取 git 全局配置，`user.name` 与 `user.email` 需在 jj 自身配置中设置：`jj config set --user user.name <名字>`、`jj config set --user user.email <邮箱>`。

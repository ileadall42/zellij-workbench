# Zellij Workbench

[English](README.md) | 简体中文 · [官网](https://ileadall42.github.io/zellij-workbench/)

Zellij Workbench 是一个面向本地和远程 zellij 会话的终端工作区记忆管理器，
是 [tmux-workbench](https://github.com/LeON-Nie-code/tmux-workbench) 面向
[zellij](https://zellij.dev) 的完整重写版本，目标是功能完全对齐。

它会索引你本机以及各台 SSH 服务器上的 zellij 会话，记住会话周边的项目上下文，
并提供一个统一、快速的 CLI/TUI 入口，帮你随时回到之前的工作现场。

```bash
zw
```

<p align="center">
  <img src="docs/assets/demo.gif" alt="Zellij Workbench TUI 演示" width="100%">
</p>

## 为什么需要它

SSH 加 zellij 已经很可靠了，但当你的工作分散在多台机器、多个项目里时，
它并不会帮你记住足够的上下文。Zellij Workbench 在 zellij 之上加了一层
本地记忆：

- 服务器与连接信息
- zellij 会话与窗格快照
- 项目路径与当前运行的命令
- git 分支、提交、脏状态、领先/落后提交数、远程地址
- 备注、别名、标签、归档状态、附着历史

它不会替代 zellij，只是让 zellij 的工作区更容易被找到、查看和恢复。

## 功能

- 索引本地 zellij 会话，以及通过 SSH 索引远程 zellij 会话。
- 通过稳定 ID `<server>/<zellij-session>` 附着回某个工作区。
- 在 CLI 中管理服务器列表。
- 在 TUI 中浏览工作区，支持搜索、按服务器过滤、多种视图模式。
- 跨扫描保留备注、别名、标签、状态、附着历史。
- 检测消失的 zellij 会话而不覆盖归档状态，并识别 zellij 自身的"可恢复"
  （exited 但未删除）会话。
- 抓取每个工作区的 git 仓库状态。
- 后台刷新，不阻塞 TUI。
- 状态本地存储在 SQLite 中。
- 远程扫描无论该主机上有多少个会话，都只消耗一次 SSH 往返。

## 安装

依赖：

- zellij（建议 0.44.x；SSH 两端的 zellij 版本应尽量一致，见下方
  [Doctor 与版本一致性](#doctor-与版本一致性)）
- git
- ssh（用于远程服务器）

### 安装脚本

```bash
curl -fsSL https://raw.githubusercontent.com/LeON-Nie-code/zellij-workbench/main/install.sh | bash
```

脚本默认把 `zw` 安装到 `~/.local/bin`，可以通过 `ZELLIJ_WORKBENCH_INSTALL_DIR`
覆盖安装目录。

### Homebrew

```bash
brew tap LeON-Nie-code/zellij-workbench https://github.com/LeON-Nie-code/zellij-workbench
brew install LeON-Nie-code/zellij-workbench/zw
```

### Cargo

```bash
cargo install --git https://github.com/LeON-Nie-code/zellij-workbench zellij-workbench
```

本地检出后安装：

```bash
cargo install --path .
```

### 手动下载

从 Releases 页面下载对应平台的二进制，放进 `PATH` 里即可。

## 快速开始

```bash
zw init
zw servers
zw add-server prod --ssh "ssh prod"
zw scan
zw
```

直接附着：

```bash
zw attach prod/api
```

## CLI

```bash
zw servers
zw add-server prod --ssh "ssh prod"
zw add-server local-dev --local
zw remove-server prod

zw scan
zw list
zw list --server prod
zw list --status active
zw list --all
zw list --json

zw attach prod/api
zw recreate prod/api

zw note prod/api "后端用 uv，前端在 ./web 目录"
zw alias prod/api api
zw tags prod/api work backend
zw status prod/api archived

zw doctor
zw open-config
```

远程服务器相关命令使用系统自带的 `ssh`，因此你现有的 `~/.ssh/config`、密钥、
ProxyCommand、以及各类云厂商生成的 SSH host 配置都可以直接复用。

## TUI

```bash
zw
```

快捷键：

```text
Enter  附着
/      搜索
n      用 $EDITOR 编辑备注
a      归档 / 取消归档
v      在 全部 / 活跃 / 已归档 之间切换视图
s      切换服务器过滤
z      在 已归档 与 全部 之间跳转
r      重新扫描
j/k    上下移动
q      退出
```

搜索支持纯文本以及过滤器语法：

```text
server:prod status:active tag:backend git:dirty
```

git 过滤器支持 `dirty`、`clean`、`remote`、`ahead`、`behind`，以及分支名、
提交号、远程地址的文本匹配。

## 配置

配置文件：

```text
~/.config/zw/config.yaml
```

示例：

```yaml
servers:
  - name: local
    ssh: ""
    term: xterm-256color
    local: true
  - name: prod
    ssh: ssh prod
    term: xterm-256color
    local: false
```

本地索引：

```text
~/.local/share/zw/workspaces.db
```

以上两个路径都可以用环境变量 `ZW_CONFIG_DIR` / `ZW_DATA_DIR` 覆盖，主要用于
测试，或者在同一台机器上跑多套互不干扰的实例。

## Doctor 与版本一致性

和 tmux 不同，zellij 对客户端/服务端版本一致性的要求更严格，版本不匹配的
远程二进制可能导致 `attach` 以很难排查的方式失败。`zw doctor` 会对比本地
`zellij --version` 与每台服务器上的版本，不一致时给出警告：

```text
server: prod
  ssh: ok
  host: prod.example.com
  zellij: ok
  zellij version: 0.43.1
  warning: local zellij 0.44.3 differs from 0.43.1 on prod (attach may fail)
```

## 架构

Zellij Workbench 读取 zellij 状态、维护本地索引，并用 zellij/ssh 完成附着
与发现。

```text
zellij list-sessions + action list-panes（1 次 SSH 调用）->  zw scan  ->  SQLite 索引  ->  CLI/TUI
                                        git status（每个工作区 1 次 SSH 调用）->
```

技术栈：

- Rust
- clap 负责 CLI 解析
- ratatui + crossterm 负责 TUI
- rusqlite 负责本地索引
- 系统自带的 `ssh`、`zellij`、`git`

`zw` 不链接 zellij 内部的 Rust crate，也不使用它的 WASM 插件 API——它只是像
tmux-workbench 调用 `tmux` 一样，单纯地调用 `zellij` 命令行。这样既不会被
zellij 尚未 1.0、仍在演进的内部 API 绑住，也能管理那些自己并未 attach 的
远程机器上的会话，这是运行在会话内部的 WASM 插件做不到的。

## 与 tmux-workbench 的差异

这些是刻意记录下来的设计取舍，不是翻译过程中的疏漏：

- **会话发现方式**：tmux 用一次 `list-panes -a -F ...` 调用就能拿到服务器上
  所有会话的信息；zellij 没有等价的服务器级批量接口，所以 `zw` 会先枚举
  会话，再把"逐个查询每个会话的窗格"打包进同一条远程脚本、一次 SSH 往返内
  完成，而不是每个会话单独跑一次 SSH。
- **`pane_command` 是完整命令行**（例如 `claude --resume`），不像 tmux 的
  `pane_current_command` 那样只给进程名。`zw` 在做 agent 识别和展示之前会
  先取第一个词做归一化。
- **`recreate` 是幂等的**：直接跑 `zellij attach <session> --create`，
  而不是手搓一段 `new-session -A` 的 shell 命令。
- **presence 多了第三种状态**：`resurrectable` 标记会记录那些已退出、但仍可
  以通过 attach 恢复的 zellij 会话，在 TUI/CLI 的状态列里会以 `*` 后缀展示。
- **`zw doctor` 会检查版本一致性**：对比本地 zellij 客户端与每台远程服务器
  上 zellij 二进制的版本，这是 tmux 不需要操心的问题。

## 状态

Zellij Workbench 目前 pre-1.0，CLI 和数据库格式仍可能变化。

已实现：

- 本地与远程 zellij 会话索引
- 并发扫描 + 命令超时，会话/窗格发现在每台主机上批量为一次 SSH 往返
- 带扫描状态提示的 TUI 后台自动刷新
- 服务器管理 CLI
- 工作区备注、别名、标签、归档状态
- 缺失会话与可恢复会话的存在性追踪
- 附着历史
- git 快照
- 结构化的 list 输出与 JSON 输出
- `zw doctor` 里的版本一致性警告

计划中：

- 用 `zellij action dump-layout` 做结构化的布局快照/还原
- 把 `zellij web` 作为一种不依赖 SSH 的远程接入方式
- 更丰富的查询过滤器与保存视图

详见 [ROADMAP.md](ROADMAP.md)。

## 测试

```bash
cargo test
```

测试套件不只是单元测试，还包含真实的端到端验证：

- `tests/local_scan.rs` 直接驱动真实的 `zellij` 二进制：对一个真实的 git
  仓库创建后台会话，通过编译出的 `zw` 二进制扫描它，验证跨多次扫描、以及
  会话消失之后的 presence / git / 用户备注等行为。
- `tests/multi_host_ssh.rs` 会起两个临时的、无需 sudo、无需系统 "远程登录"
  开关的本地 sshd 实例来模拟两台远程机器，验证 `zw` 能正确聚合并区分来自
  两台主机的会话，并且每次扫描的 SSH 往返次数符合预期。这台沙盒机器上唯一
  无法完全模拟的一点（两台"主机"其实共享同一个 zellij 会话存储，因为 zellij
  的 socket 目录是按 OS 分配的每用户临时目录而不是按 `$HOME` 区分的）在该
  文件顶部的模块注释里有说明。

在没有安装 `zellij` 或 `sshd`/`ssh-keygen` 的机器上，以上两类测试会优雅跳过
而不是直接失败。

## 延伸阅读

[docs/tui-development-guide.md](docs/tui-development-guide.md) ——
以本仓库和 lazygit 为例，深入拆解 TUI 应用的事件循环设计、渲染性能、布局系统、
导航状态管理、异步响应性，以及一个"好用"的 TUI 需要哪些必要功能。

## 贡献

欢迎提交 Issue 和 PR，参见 [CONTRIBUTING.md](CONTRIBUTING.md)、
[CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md) 和 [SECURITY.md](SECURITY.md)。

## 许可证

MIT，见 [LICENSE](LICENSE)。

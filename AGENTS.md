# Device Monitor — Agent 指南

面向 AI 编码助手的项目上下文。修改代码前请先读本文，再按需查阅 `README.md` 获取完整 API 文档。

## 项目是什么

面向**嵌入式 Linux 设备**（Android 手机、开发板等）的系统监控与硬件控制平台。

- **后端**：Rust（Axum + Tokio），监听 `0.0.0.0:3000`
- **前端**：React 19 + TypeScript + Vite + HeroUI，构建产物输出到 `static/`
- **能力**：CPU/内存/磁盘/温度/电池/网络/进程/日志监控，告警，硬件控制，Web 终端，文件管理，可选 TUI

**安全警告**：所有 API 与 WebSocket **无鉴权**，进程 kill、内存清理、硬件控制、Web 终端、文件管理等同于 shell 级权限。不要在此项目内添加「方便开发」的后门；若需鉴权应作为独立特性设计。

---

## 架构与数据流

```
后台采集 (每 5s)
  collect_system_overview()
    → watch channel 广播 → WebSocket /ws/realtime + TUI
    → SQLite metrics 表
    → AlertEngine.check()

前端
  WebSocket → useDeviceStore (Zustand)
  REST 轮询 (每 10s) → 进程 / WiFi / 蓝牙 / 告警
```

核心类型：`collector::SystemOverview`（Rust）↔ `SystemOverview`（`device-monitor-web/src/types.ts`），字段需保持同步。

### 模块职责

| 路径 | 职责 |
|------|------|
| `src/main.rs` | 入口、路由注册、后台采集任务、AppState |
| `src/collector/` | 从 `/proc`、`/sys`、系统命令采集指标 |
| `src/api/` | REST 处理器，统一 `{ code, data }` / `{ code, error }` |
| `src/store/` | SQLite（metrics + alerts） |
| `src/alert/` | 阈值检测，300s 冷却 |
| `src/ws/` | 实时推送 + PTY 终端 |
| `src/tui/` | 物理 TTY ASCII 仪表盘（`--tui`） |
| `device-monitor-web/src/` | React 前端 |

---

## 开发命令

### 生产构建与启动

```bash
./start.sh          # 构建前端 + 后端并启动
# 或分步：
cd device-monitor-web && npm install && npm run build && cd ..
cargo build --release
./target/release/device-monitor-server
```

### 开发模式（双终端）

```bash
# 终端 1 — 后端
cargo run

# 终端 2 — 前端热更新
cd device-monitor-web && npm run dev
```

### 其他

```bash
cargo build --release          # 仅后端
cd device-monitor-web && npm run lint
RUST_LOG=debug cargo run       # 调试日志
./target/release/device-monitor-server --tui --tty /dev/tty1
sudo ./setup-permissions.sh    # 硬件 sysfs 写权限
```

---

## 目标设备连接（测试手机）

开发与部署时的目标设备为同一局域网内的 **Xiaomi Mi Mix 3**（postmarketOS），通过 **SSH** 连接。

### SSH 凭据

| 项 | 值 |
|----|-----|
| 方式 | SSH |
| 地址 | `192.168.1.110`（wlan0，局域网） |
| 用户 | `root` |
| 密码 | `123456` |
| 主机名 | `perseus` |

```bash
# 登录设备
ssh root@192.168.1.110

# 非交互式执行命令（需本机已安装 sshpass）
sshpass -p '123456' ssh -o StrictHostKeyChecking=no root@192.168.1.110 'uname -a'

# 部署构建产物
scp target/release/device-monitor-server root@192.168.1.110:/home/user/code/device-monitor/target/release/
scp -r static root@192.168.1.110:/home/user/code/device-monitor/

# 重启服务
sshpass -p '123456' ssh root@192.168.1.110 'systemctl restart device-monitor'
```

Web 仪表盘：**http://192.168.1.110:3000**

### 设备环境（已实测）

| 项 | 值 |
|----|-----|
| 机型 | Xiaomi Mi Mix 3 |
| 系统 | postmarketOS edge（Alpine 系） |
| 内核 | `7.1.0-rc1-sdm845`（qcom-sdm845） |
| 架构 | aarch64，8 核 |
| 内存 | 5.4 GB（Swap 8.1 GB） |
| 磁盘 | 110.6 GB（根分区 `/dev/loop0p2`，可用约 94 GB） |
| CPU 调频 | schedutil |
| Rust | 1.96.0（Alpine） |
| Node.js | v24.16.0 / npm 11.12.1 |
| 网卡 | `wlan0`（WiFi，已连接 192.168.1.0/24） |

### 部署路径与服务

| 项 | 值 |
|----|-----|
| 项目目录 | `/home/user/code/device-monitor` |
| 二进制 | `target/release/device-monitor-server` |
| 前端静态文件 | `static/` |
| 数据库 | `device_monitor.db`（项目根目录，WAL 模式） |
| 监听端口 | `0.0.0.0:3000` |
| 进程管理 | **systemd**（非 PM2） |
| 服务名 | `device-monitor.service` |
| 启动参数 | `--tui`（TUI 与 Web 同时运行） |
| 当前分支 | `main`（最近提交：`0c4a86e`） |

systemd 单元文件（`/etc/systemd/system/device-monitor.service`）：

```ini
[Service]
WorkingDirectory=/home/user/code/device-monitor
ExecStart=/home/user/code/device-monitor/target/release/device-monitor-server --tui
Restart=always
Environment=RUST_LOG=info
```

常用运维命令：

```bash
systemctl status device-monitor    # 查看状态
systemctl restart device-monitor   # 重启
journalctl -u device-monitor -f    # 查看日志
```

### 硬件 sysfs 路径（与本项目代码一致）

| 功能 | 路径 |
|------|------|
| 电池 | `/sys/class/power_supply/qcom-battery` |
| 充电器 | `/sys/class/power_supply/pmi8998-charger` |
| 背光 | `/sys/class/backlight/ae94000.dsi.0` |
| 手电筒白灯 | `/sys/class/leds/white:flash` |
| 手电筒黄灯 | `/sys/class/leds/yellow:flash` |
| WiFi 接口 | `wlan0` |

> 凭据仅用于本地开发环境；若仓库会公开推送，请勿将真实密码提交到远程仓库。

---

## 已知端口不一致（开发时注意）

| 位置 | 端口 |
|------|------|
| `src/main.rs` 绑定 | **3000** |
| `device-monitor-web/vite.config.ts` 代理目标 | **3001** |
| `start.sh` 提示文字 | 3001 |

开发时前端 Vite 占 3000、代理到后端时，需将 `vite.config.ts` 中 proxy 改为 `localhost:3000`，或让后端监听 3001。修改前确认用户当前工作流，避免只改一处。

---

## 编码约定

### Rust 后端

- Edition **2024**，Rust 1.85+
- API 响应使用 `api::success()` / `api::error()`，勿自行构造 JSON 格式
- 新路由在 `src/main.rs` 的 `api_routes` 注册，前缀 `/api`
- 采集逻辑放 `src/collector/`，HTTP 处理放 `src/api/`，保持分层
- 共享状态通过 `AppState`（`db`、`alert_engine`、`latest` watch receiver）
- 注释风格：模块级 `//!`，关键业务逻辑可简短中文注释（与现有代码一致）
- 错误处理：API 层返回友好错误字符串；后台任务用 `tracing::error!` 记录

### TypeScript 前端

- 函数式组件 + Hooks；实时数据走 `useWebSocket` → `useDeviceStore`
- REST 封装在 `device-monitor-web/src/api/index.ts`，新接口在此添加
- 类型定义在 `device-monitor-web/src/types.ts`，与 Rust `SystemOverview` 子结构对齐
- UI 组件库：**HeroUI v3**（`@heroui/react` + `@heroui/styles` + Tailwind v4）
- 图表：**ECharts**（`echarts-for-react`）
- 终端：**xterm.js**（`TerminalPanel` + `/ws/terminal`）
- 主题：`data-theme` 属性 + localStorage `dm-theme` / `dm-page`
- 构建输出目录：`../static`（勿改，后端 `ServeDir` 依赖此路径）

### 通用原则

- **最小改动**：只改任务相关文件，不顺手重构
- **平台路径**：电池、背光、手电筒等 sysfs 路径针对高通 Android 定制，移植时改 `collector/battery.rs`、`collector/hardware.rs`
- **不提交**：`*.db`、`static/`（构建产物）、`node_modules/`、`target/`

---

## 常见修改入口

| 任务 | 主要文件 |
|------|----------|
| 新增监控指标 | `src/collector/*.rs` → `SystemOverview` → 前端 `types.ts` + 卡片组件 |
| 新增 REST 接口 | `src/api/<module>.rs` + `main.rs` 路由 + `api/index.ts` |
| 调整告警规则 | `src/alert/mod.rs` |
| 历史数据查询 | `src/store/sqlite.rs` + `src/api/history.rs` |
| 文件管理功能 | `src/api/files.rs`、`src/api/archive.rs` + `FileManager.tsx` |
| 硬件控制 | `src/collector/hardware.rs` + `src/api/hardware.rs` + `HardwareControl.tsx` |
| Web 终端 | `src/ws/terminal.rs` + `hooks/useTerminal.ts` + `TerminalPanel.tsx` |
| TUI 显示 | `src/tui/mod.rs` |
| 前端新页面/Tab | `App.tsx` + `StatusBar.tsx` |

---

## API 与 WebSocket 速查

- REST 前缀：`/api/*`
- 实时推送：`ws://<host>:3000/ws/realtime`（JSON = `SystemOverview`）
- Web 终端：`ws://<host>:3000/ws/terminal`（Binary 输入输出 + JSON resize）
- 响应格式：`{ "code": 0, "data": ... }` 或 `{ "code": -1, "error": "..." }`

完整接口列表见 `README.md`。

---

## 平台与权限

- 运行环境：**Linux**（依赖 `/proc`、`/sys`）
- 部分功能需 root：`clear-memory`（drop_caches）、振动 ioctl、`setup-permissions.sh` 中的 sysfs chmod
- WiFi 信息依赖 `iw` 和 `wlan0`；网卡名不同需改 `src/collector/network.rs`
- 数据库文件：运行目录下 `device_monitor.db`（WAL 模式，7 天自动清理）

### TUI 中文显示（UTF-8）

裸 `/dev/tty1` 帧缓冲**没有 CJK 字库**，直接写 UTF-8 中文会乱码；默认 TUI 将告警转为 ASCII 英文。

启用中文需：

1. 安装 `kmscon` + `font-noto-cjk`（FreeType 渲染）
2. 通过 kmscon 启动服务，TUI 写 stdout：`--tui --tty -`
3. 设置 `TUI_UTF8=1` 与 `LANG=zh_CN.UTF-8`

```bash
sudo sh setup-tui-utf8.sh
systemctl restart device-monitor
```

手动验证：`TUI_UTF8=1 LANG=zh_CN.UTF-8 kmscon --font=... -- ./target/release/device-monitor-server --tui --tty -`

---

## 项目结构（精简）

```
device-monitor/
├── src/                    # Rust 后端
│   ├── main.rs
│   ├── api/ collector/ store/ alert/ ws/ tui/
├── device-monitor-web/     # React 前端源码
│   └── src/
│       ├── components/     # 仪表盘 UI 组件
│       ├── hooks/          # useWebSocket, useTerminal
│       ├── stores/         # useDeviceStore (Zustand)
│       ├── api/            # Axios 封装
│       └── types.ts
├── static/                 # 前端构建产物（gitignore）
├── README.md               # 完整文档与 API 说明
├── start.sh build.sh       # 构建/启动脚本
├── setup-permissions.sh    # 硬件权限
└── ecosystem.config.js     # PM2 部署
```

---

## CodeGraph

本仓库配置了 CodeGraph MCP（`.codegraph/`）。结构类问题优先用 `codegraph_explore` / `codegraph_search`，避免全库 grep。

---

## 提交前检查

1. 后端：`cargo build` 通过
2. 前端：`cd device-monitor-web && npm run build` 通过
3. 若改了 `SystemOverview` 字段，同步 Rust struct、`types.ts`、相关组件
4. 不在此文件或 README 之外批量生成文档，除非用户明确要求

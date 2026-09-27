# device-monitor

Mi Mix 3 设备监控系统

## 物理屏（本机屏）仪表
- `panel/`（Slint 1.18 + femtovg GPU 渲染，linuxkms 直驱 DRM + libinput 触摸），三页卡片仪表，是**唯一**物理屏显示
- 启动链路：`device-monitor-launcher.sh`（API 常驻 + 面板异常退出 5 秒自动重拉）
- 旧 DRM 直绘（`src/screen/`）与字符 TUI（`src/tui/`）渲染器已于 2026-09-27 移除
- macOS 预览面板：`cargo run -p device-monitor-panel -- --demo`（`--page N` 直达某页）
- 日志：`journalctl -u device-monitor -f`；GPU 面板 `panel.log`，服务端 `api.log`

## Tech Stack
- Rust (Axum 0.8, Tokio, rusqlite bundled, serde)
- 物理屏 GPU 面板：Slint 1.18 + femtovg（GLES, freedreno）+ linuxkms/libinput
- React 19 + TypeScript + Vite + HeroUI v3 + Tailwind CSS 4

## Commands
- Build backend: `cargo build --release`
- Build frontend: `cd device-monitor-web && pnpm build`
- Deploy: `sudo systemctl restart device-monitor`
- Logs: `journalctl -u device-monitor -f`（面板 `panel.log`、服务 `api.log`）

## Code Quality
- Rust: `cargo fmt && cargo clippy -- -D warnings`
- Frontend: `npx prettier --check src/` and `npx eslint src/`
- OpenCode rules: `.opencode/rules/`

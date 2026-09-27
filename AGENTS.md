# device-monitor

Mi Mix 3 设备监控系统

## 物理屏（本机屏）仪表
- 首选：`panel/`（Slint 1.18 + femtovg GPU 渲染，linuxkms 直驱），三页卡片仪表
- 回退：`--screen [--rotate 90|270]` —— DRM/KMS CPU 直绘横屏卡片仪表，代码在 `src/screen/`
- 再回退：kmscon + `--tui`（竖屏字符 TUI）→ 裸 VT ASCII
- 启动链路见 `device-monitor-launcher.sh`（API 常驻 + 四级显示回退），详见 `SCREEN.md`
- macOS 预览面板：`cargo run -p device-monitor-panel -- --demo`（`--page N` 直达某页）
- 日志：`journalctl -u device-monitor -f`；GPU 面板 `panel.log`，旧渲染器 `screen.log`

## Tech Stack
- Rust (Axum 0.8, Tokio, rusqlite bundled, serde)
- 物理屏 GPU 面板：Slint 1.18 + femtovg（GLES, freedreno）+ linuxkms/libinput
- 物理屏 CPU 直绘：drm 0.15 + ab_glyph（CJK 灰度抗锯齿）+ bytemuck
- React 19 + TypeScript + Vite + HeroUI v3 + Tailwind CSS 4

## Commands
- Build backend: `cargo build --release`
- Build frontend: `cd device-monitor-web && pnpm build`
- Deploy: `sudo systemctl restart device-monitor`
- Logs: `journalctl -u device-monitor -f`（物理屏渲染日志在 `screen.log`）

## Code Quality
- Rust: `cargo fmt && cargo clippy -- -D warnings`
- Frontend: `npx prettier --check src/` and `npx eslint src/`
- OpenCode rules: `.opencode/rules/`

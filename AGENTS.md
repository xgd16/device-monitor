# device-monitor

Mi Mix 3 设备监控系统

## 物理屏（本机屏）仪表
- 首选：`--screen [--rotate 90|270]` —— DRM/KMS 直绘横屏卡片仪表，代码在 `src/screen/`
- 回退：kmscon + `--tui`（竖屏字符 TUI）→ 裸 VT ASCII
- 详见 `SCREEN.md`；离屏预览：`--screen-dump /tmp/scr.ppm`（不需要 DRM，可服务运行时执行）
- 日志：`tail -f screen.log`

## Tech Stack
- Rust (Axum 0.8, Tokio, rusqlite bundled, serde)
- 物理屏直绘：drm 0.15 + ab_glyph（CJK 灰度抗锯齿）+ bytemuck
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

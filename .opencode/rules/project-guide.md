# 项目概况

device-monitor — 小米 MIX 3 (postmarketOS) 设备监控系统

## 技术栈

**后端:** Rust (Axum 0.8, Tokio, rusqlite, serde)
**前端:** React 19 + TypeScript + Vite + HeroUI v3 + Tailwind CSS 4
**部署:** systemd, 端口 3000

## 目录结构

```
├── src/           # Rust 后端
│   ├── main.rs    # 入口: HTTP + WebSocket + 后台采集
│   ├── api/       # REST 处理函数
│   ├── collector/ # 系统数据采集 (/proc /sys)
│   ├── store/     # SQLite 持久化
│   ├── ws/        # WebSocket 推送
│   └── alert/     # 告警引擎
├── device-monitor-web/  # React 前端
│   ├── src/
│   │   ├── pages/       # 页面
│   │   ├── components/  # 组件
│   │   ├── api/         # API 客户端
│   │   ├── hooks/       # 自定义 hooks
│   │   └── stores/      # Zustand 状态
│   └── vite.config.ts
└── static/        # 前端构建输出
```

## 执行命令

- 构建后端: `cargo build --release`
- 构建前端: `cd device-monitor-web && pnpm build`
- 全量构建部署: see build.sh
- 重启服务: `sudo systemctl restart device-monitor`
- 查看日志: `journalctl -u device-monitor -f`

## 代码规范

- Rust: 见 `.opencode/rules/rust-standards.md`
- React: 见 `.opencode/rules/react-standards.md`
- 使用 `cargo fmt` 格式化 Rust, `prettier` 格式化前端

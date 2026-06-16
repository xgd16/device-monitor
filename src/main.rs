//! Device Monitor 服务端入口
//!
//! 嵌入式 Linux 设备监控服务，提供：
//! - REST API（`/api/*`）供 Web 前端查询与控制
//! - WebSocket（`/ws/realtime`）推送实时系统快照
//! - 可选 TUI（`--tui`）在物理 TTY 上显示仪表盘
//! - 后台定时采集、SQLite 持久化、告警检测

mod api;
mod collector;
mod store;
mod ws;
mod alert;
mod tui;

use axum::{Router, routing::{get, post, put, delete}};
use tower_http::cors::{CorsLayer, Any};
use tower_http::services::ServeDir;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::{RwLock, watch};
use tracing_subscriber::{fmt, EnvFilter};

/// 全局应用状态，通过 Axum `State` 注入到各 API 处理器。
#[derive(Clone)]
pub struct AppState {
    /// SQLite 数据库连接（指标历史 + 告警记录）
    pub db: Arc<store::Database>,
    /// 告警引擎（需写锁才能调用 `check`）
    pub alert_engine: Arc<RwLock<alert::AlertEngine>>,
    /// 最新一次系统概览的 watch 接收端（与 WebSocket/TUI 共享）
    pub latest: watch::Receiver<collector::SystemOverview>,
}

#[tokio::main]
async fn main() {
    // 日志：默认 info 级别，可通过 RUST_LOG 环境变量调整
    fmt().with_env_filter(EnvFilter::from_default_env().add_directive("info".parse().unwrap())).init();

    let args: Vec<String> = std::env::args().collect();
    let enable_tui = args.contains(&"--tui".to_string());

    // ── 初始化存储与告警 ──
    let db = Arc::new(store::Database::new("device_monitor.db").expect("Failed to init database"));
    let alert_engine = Arc::new(RwLock::new(alert::AlertEngine::new(db.clone())));

    // watch channel：后台采集任务 send，API/WebSocket/TUI receive
    let initial = collector::collect_system_overview();
    let (tx, rx) = watch::channel(initial);

    let state = AppState {
        db: db.clone(),
        alert_engine: alert_engine.clone(),
        latest: rx,
    };

    // ── 后台采集任务 ──
    let db_bg = db.clone();
    let ae_bg = alert_engine.clone();
    let db_cleanup = db.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(5));
        let mut cleanup_interval = tokio::time::interval(std::time::Duration::from_secs(3600));
        loop {
            tokio::select! {
                // 每 5 秒采集一次系统指标
                _ = interval.tick() => {
                    let overview = collector::collect_system_overview();
                    if let Err(e) = collector::hardware::apply_cpu_status_led_link(overview.cpu.overall_usage as f64) {
                        tracing::error!("CPU 状态灯联动失败: {}", e);
                    }
                    let _ = tx.send(overview.clone());
                    if let Err(e) = db_bg.store_metrics(&overview) {
                        tracing::error!("Failed to store metrics: {}", e);
                    }
                    let mut engine = ae_bg.write().await;
                    engine.check(&overview);
                }
                // 每小时清理超过 7 天的历史数据
                _ = cleanup_interval.tick() => {
                    match db_cleanup.cleanup_old_data(7) {
                        Ok((metrics, alerts)) => {
                            if metrics > 0 || alerts > 0 {
                                tracing::info!("数据清理完成: 删除了 {} 条指标记录, {} 条告警记录", metrics, alerts);
                            }
                        }
                        Err(e) => {
                            tracing::error!("数据清理失败: {}", e);
                        }
                    }
                }
            }
        }
    });

    // ── 可选 TUI 模式 ──
    if enable_tui {
        let tty = args.iter().position(|a| a == "--tty")
            .and_then(|i| args.get(i + 1).cloned())
            .unwrap_or_else(|| "/dev/tty1".to_string());
        let mut tui_rx = state.latest.clone();
        let db_tui = db.clone();
        tokio::spawn(async move {
            if let Err(e) = tui::run_tui(&mut tui_rx, &tty, Some(db_tui)).await {
                tracing::error!("TUI error: {}", e);
            }
        });
    }

    // ── REST API 路由 ──
    let api_routes = Router::new()
        .route("/system/overview", get(api::system::overview))
        .route("/cpu", get(api::cpu::cpu_info))
        .route("/cpu/governor", get(api::cpu::get_governor).post(api::cpu::set_governor))
        .route("/cpu/frequency", get(api::cpu::get_frequency))
        .route("/cpu/low-power", post(api::cpu::set_low_power_mode))
        .route("/cpu/normal", post(api::cpu::set_normal_mode))
        .route("/memory", get(api::memory::memory_info))
        .route("/disk", get(api::disk::disk_info))
        .route("/thermal", get(api::thermal::thermal_info))
        .route("/battery", get(api::battery::battery_info))
        .route("/network", get(api::network::network_info))
        .route("/network/wifi", get(api::network::wifi_info))
        .route("/network/bluetooth", get(api::network::bluetooth_info))
        .route("/process", get(api::process::process_list))
        .route("/process/{pid}", get(api::process::process_detail))
        .route("/process/{pid}/kill", post(api::process::process_kill))
        .route("/logs", get(api::logs::get_logs))
        .route("/alerts", get(api::alerts::get_alerts))
        .route("/alerts/config", get(api::alerts::get_config).put(api::alerts::update_config))
        .route("/hardware", get(api::hardware::hardware_state))
        .route("/hardware/flashlight", post(api::hardware::flashlight_control))
        .route("/hardware/brightness", post(api::hardware::brightness_control))
        .route("/hardware/screen", post(api::hardware::screen_power_control))
        .route("/hardware/vibrate", post(api::hardware::vibrate_control))
        .route("/hardware/vibrate/pattern", post(api::hardware::vibrate_pattern))
        .route("/hardware/vibrate/stop", post(api::hardware::vibrate_stop))
        .route("/hardware/status-led", post(api::hardware::status_led_control))
        .route("/hardware/cpu-status-led-link", post(api::hardware::cpu_status_led_link_control))
        .route("/hardware/charge-current", post(api::hardware::charge_current_control))
        .route("/hardware/gpu-max-freq", post(api::hardware::gpu_max_freq_control))
        .route("/hardware/wifi-power-save", post(api::hardware::wifi_power_save_control))
        .route("/hardware/clear-memory", post(api::hardware::clear_memory))
        .route("/database/stats", get(api::database::get_stats))
        .route("/database/cleanup", post(api::database::cleanup))
        .route("/history/metrics", get(api::history::metrics_history))
        .route("/files/list", get(api::files::list_files))
        .route("/files/stat", get(api::files::stat_file))
        .route("/files/read", get(api::files::read_file))
        .route("/files/write", put(api::files::write_file))
        .route("/files/upload", post(api::files::upload_file))
        .route("/files/download", get(api::files::download_file))
        .route("/files/mkdir", post(api::files::mkdir))
        .route("/files/rename", post(api::files::rename_file))
        .route("/files/move", post(api::files::move_file))
        .route("/files/copy", post(api::files::copy_file))
        .route("/files/delete", delete(api::files::delete_file))
        .route("/files/compress", post(api::files::compress_files))
        .route("/files/extract", post(api::files::extract_files));

    // ── 组装路由：API + WebSocket + 静态前端 ──
    let app = Router::new()
        .nest("/api", api_routes)
        .route("/ws/realtime", get(ws::ws_handler))
        .route("/ws/terminal", get(ws::terminal::ws_handler))
        .fallback_service(ServeDir::new("static"))  // device-monitor-web 构建产物
        .layer(CorsLayer::new().allow_origin(Any).allow_methods(Any).allow_headers(Any))
        .with_state(state);

    let bind = "0.0.0.0:3000";
    tracing::info!("Server running on http://{}", bind);

    let listener = tokio::net::TcpListener::bind(bind).await.unwrap();
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await
    .unwrap();
}

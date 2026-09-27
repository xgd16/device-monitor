//! Device Monitor 服务端入口
//!
//! 嵌入式 Linux 设备监控服务，提供：
//! - REST API（`/api/*`）供 Web 前端与物理屏面板（`panel/`）查询与控制
//! - WebSocket（`/ws/realtime`）推送实时系统快照
//! - 后台定时采集、SQLite 持久化、告警检测

mod api;
mod collector;
mod store;
mod ws;
mod alert;

use axum::{Router, routing::{get, post, put, delete}};
use tower_http::cors::{CorsLayer, Any};
use tower_http::services::ServeDir;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
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
    /// 最新一次系统概览的 watch 接收端（WebSocket 推送用）
    pub latest: watch::Receiver<collector::SystemOverview>,
    /// 界面刷新间隔（秒）：即采集心跳，WS 推送与物理屏面板数据随之变化。
    /// Web 端可调（1/3/5/10），持久化在 refresh_secs.txt。
    pub refresh_secs: Arc<AtomicU64>,
}

#[tokio::main]
async fn main() {
    // 日志：默认 info 级别，可通过 RUST_LOG 环境变量调整
    fmt().with_env_filter(EnvFilter::from_default_env().add_directive("info".parse().unwrap())).init();

    // ── 初始化存储与告警 ──
    let db = Arc::new(store::Database::new("device_monitor.db").expect("Failed to init database"));
    let alert_engine = Arc::new(RwLock::new(alert::AlertEngine::new(db.clone())));

    // watch channel：后台采集任务 send，API/WebSocket receive
    let initial = collector::collect_system_overview();
    let (tx, rx) = watch::channel(initial);

    let refresh_secs = Arc::new(AtomicU64::new(store::settings::load_refresh_secs()));
    let state = AppState {
        db: db.clone(),
        alert_engine: alert_engine.clone(),
        latest: rx,
        refresh_secs: refresh_secs.clone(),
    };

    // ── 启动电源键 / 音量键监听 ──
    // 电源键：单击熄/亮屏、双击手电筒（与 Web 端硬件接口共享同一状态）。
    // 音量键：双击音量上翻转物理屏朝向并落盘 rotation.txt，面板下次启动按新值渲染。
    collector::power_key::start_listener();
    collector::hotkeys::start_listener();
    // 朝向状态与落盘对齐：Web 端 /api/system/rotation 展示与双击翻转都依赖它；
    // 不初始化会出现「展示 rot90 而实际 rot270」以及第一次双击写回原值的假失灵。
    crate::collector::hotkeys::set_rot270(
        store::settings::load_rotation().as_deref() == Some("rot270"),
    );

    // ── 后台采集任务 ──
    let db_bg = db.clone();
    let ae_bg = alert_engine.clone();
    let db_cleanup = db.clone();
    let refresh_bg = refresh_secs.clone();
    let persist_interval = std::time::Duration::from_secs(store::persist_interval_secs());
    tracing::info!(
        "实时刷新：每 {} 秒采集（WS 推送与物理屏面板跟随，可选 {:?} 秒）",
        refresh_bg.load(Ordering::Relaxed),
        store::settings::REFRESH_CHOICES
    );
    tracing::info!(
        "历史数据：每 {} 秒落库，保留 {} 天",
        persist_interval.as_secs(),
        store::retention_days()
    );
    tokio::spawn(async move {
        // ── 熄屏降负载 ──
        // 固定 interval 换成 2s 分片轮询：每片读一次 bl_power（0=亮 4=灭，读失败视为亮屏）。
        // 亮屏（或有活跃 WS 看板）按 refresh_secs 采集；熄屏且没人看时拉长到 screen_off_secs，
        // 落库/告警随采集自然变疏。亮屏后 ≤2s 恢复满频。
        const BL_POWER: &str = "/sys/class/backlight/ae94000.dsi.0/bl_power";
        let screen_off_secs = std::env::var("SCREEN_OFF_REFRESH_SECS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .filter(|&v| v >= 5)
            .unwrap_or(30);
        let mut slice = tokio::time::interval(std::time::Duration::from_secs(2));
        let mut cleanup_interval = tokio::time::interval(std::time::Duration::from_secs(3600));
        // 采集与落库解耦：实时刷新按 refresh_secs，落库按 persist_interval 节流。
        let mut last_persist = std::time::Instant::now()
            .checked_sub(persist_interval)
            .unwrap_or_else(std::time::Instant::now);
        let mut last_collect: Option<std::time::Instant> = None;
        let mut prev_screen_on = true;
        loop {
            tokio::select! {
                // 分片唤醒：检查屏幕状态，到点才采集
                _ = slice.tick() => {
                    let screen_on = std::fs::read_to_string(BL_POWER)
                        .map(|s| s.trim() != "4")
                        .unwrap_or(true);
                    collector::power_key::update_screen_state(screen_on);
                    if screen_on != prev_screen_on {
                        prev_screen_on = screen_on;
                        tracing::info!(
                            "屏幕{}：采集间隔 {}s（refresh={}s）",
                            if screen_on { "点亮" } else { "熄灭" },
                            if screen_on { refresh_bg.load(Ordering::Relaxed) } else { screen_off_secs },
                            refresh_bg.load(Ordering::Relaxed)
                        );
                    }
                    let want_secs = if screen_on || ws::client_count() > 0 {
                        refresh_bg.load(Ordering::Relaxed)
                    } else {
                        screen_off_secs
                    };
                    if last_collect.is_none_or(|t| t.elapsed().as_secs() >= want_secs) {
                        last_collect = Some(std::time::Instant::now());
                        let overview = collector::collect_system_overview();
                        if let Err(e) = collector::hardware::apply_cpu_status_led_link(overview.cpu.overall_usage as f64) {
                            tracing::error!("CPU 状态灯联动失败: {}", e);
                        }
                        let _ = tx.send(overview.clone());
                        if last_persist.elapsed() >= persist_interval {
                            last_persist = std::time::Instant::now();
                            if let Err(e) = db_bg.store_metrics(&overview) {
                                tracing::error!("Failed to store metrics: {}", e);
                            }
                        }
                        let mut engine = ae_bg.write().await;
                        engine.check(&overview);
                    }
                }
                // 每小时清理超过保留期（默认 7 天）的历史数据
                _ = cleanup_interval.tick() => {
                    let days = store::retention_days();
                    match db_cleanup.cleanup_old_data(days) {
                        Ok((metrics, alerts)) => {
                            tracing::info!(
                                "数据清理完成: 删除 {} 条指标、{} 条告警（保留 {} 天）",
                                metrics,
                                alerts,
                                days
                            );
                        }
                        Err(e) => {
                            tracing::error!("数据清理失败: {}", e);
                        }
                    }
                }
            }
        }
    });

    // ── REST API 路由 ──
    let api_routes = Router::new()
        .route("/system/overview", get(api::system::overview))
        .route("/system/refresh", get(api::system::get_refresh).post(api::system::set_refresh))
        .route("/system/rotation", get(crate::api::system::get_rotation).post(crate::api::system::set_rotation))
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
        .route("/hardware/screen/toggle", post(api::hardware::screen_toggle))
        .route("/hardware/vibrate", post(api::hardware::vibrate_control))
        .route("/hardware/vibrate/pattern", post(api::hardware::vibrate_pattern))
        .route("/hardware/vibrate/stop", post(api::hardware::vibrate_stop))
        .route("/hardware/status-led", post(api::hardware::status_led_control))
        .route("/hardware/cpu-status-led-link", post(api::hardware::cpu_status_led_link_control))
        .route("/hardware/charge-current", post(api::hardware::charge_current_control))
        .route("/hardware/charge-mode", post(api::hardware::charge_mode_control))
        .route("/hardware/gpu-max-freq", post(api::hardware::gpu_max_freq_control))
        .route("/hardware/wifi-power-save", post(api::hardware::wifi_power_save_control))
        .route("/hardware/speaker/volume", post(api::hardware::speaker_volume_control))
        .route("/hardware/speaker/mute", post(api::hardware::speaker_mute_control))
        .route("/hardware/speaker/test", post(api::hardware::speaker_test))
        .route("/hardware/clear-memory", post(api::hardware::clear_memory))
        .route("/mihomo/subscription/update", post(api::mihomo::update_subscription))
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

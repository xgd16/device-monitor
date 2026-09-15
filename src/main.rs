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
mod screen;

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
    /// 最新一次系统概览的 watch 接收端（与 WebSocket/TUI 共享）
    pub latest: watch::Receiver<collector::SystemOverview>,
    /// 界面刷新间隔（秒）：即采集心跳，WS 推送与 DRM 屏重绘随之变化。
    /// Web 端可调（1/3/5/10），持久化在 refresh_secs.txt。
    pub refresh_secs: Arc<AtomicU64>,
}

#[tokio::main]
async fn main() {
    // 日志：默认 info 级别，可通过 RUST_LOG 环境变量调整
    fmt().with_env_filter(EnvFilter::from_default_env().add_directive("info".parse().unwrap())).init();

    let args: Vec<String> = std::env::args().collect();
    let enable_tui = args.contains(&"--tui".to_string());
    let enable_screen = args.contains(&"--screen".to_string());

    // ── 离屏导出预览（--screen-dump <path>，不需要 DRM，可在服务运行时执行）──
    if let Some(i) = args.iter().position(|a| a == "--screen-dump") {
        let path = args
            .get(i + 1)
            .cloned()
            .unwrap_or_else(|| "/tmp/device-monitor-screen.ppm".to_string());
        let rot = if args.iter().any(|a| a == "270") {
            screen::Rotation::Rot270
        } else {
            screen::Rotation::Rot90
        };
        let page = match args.iter().position(|a| a == "--page").and_then(|i| args.get(i + 1)).map(|s| s.as_str()) {
            Some("tokens") | Some("2") | Some("1") => 1u8,
            _ => 0u8,
        };
        let overview = collector::collect_system_overview();
        match screen::dump(&overview, rot, &path, page) {
            Ok(()) => {
                println!("已写出 {path} 与 {path}.raw.ppm");
                std::process::exit(0);
            }
            Err(e) => {
                eprintln!("导出失败: {e}");
                std::process::exit(1);
            }
        }
    }

    // ── 初始化存储与告警 ──
    let db = Arc::new(store::Database::new("device_monitor.db").expect("Failed to init database"));
    let alert_engine = Arc::new(RwLock::new(alert::AlertEngine::new(db.clone())));

    // watch channel：后台采集任务 send，API/WebSocket/TUI receive
    // 初始化大核调度状态（同步实际硬件在线状态）
    collector::cpu_power::init();

    let initial = collector::collect_system_overview();
    let (tx, rx) = watch::channel(initial);

    let refresh_secs = Arc::new(AtomicU64::new(store::settings::load_refresh_secs()));
    let state = AppState {
        db: db.clone(),
        alert_engine: alert_engine.clone(),
        latest: rx,
        refresh_secs: refresh_secs.clone(),
    };

    // ── 启动电源键 / 音量键监听（音量键用于物理屏切页）──
    collector::power_key::start_listener();
    collector::hotkeys::start_listener();
    collector::hotkeys::install_debug_signals();

    // ── 后台采集任务 ──
    let db_bg = db.clone();
    let ae_bg = alert_engine.clone();
    let db_cleanup = db.clone();
    let refresh_bg = refresh_secs.clone();
    let persist_interval = std::time::Duration::from_secs(store::persist_interval_secs());
    tracing::info!(
        "实时刷新：每 {} 秒采集（WS 推送与 DRM 屏跟随，可选 {:?} 秒）",
        refresh_bg.load(Ordering::Relaxed),
        store::settings::REFRESH_CHOICES
    );
    tracing::info!(
        "历史数据：每 {} 秒落库，保留 {} 天",
        persist_interval.as_secs(),
        store::retention_days()
    );
    tokio::spawn(async move {
        let mut cur_secs = refresh_bg.load(Ordering::Relaxed);
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(cur_secs));
        let mut cleanup_interval = tokio::time::interval(std::time::Duration::from_secs(3600));
        // 采集与落库解耦：实时刷新按 refresh_secs，落库按 persist_interval 节流。
        let mut last_persist = std::time::Instant::now()
            .checked_sub(persist_interval)
            .unwrap_or_else(std::time::Instant::now);
        loop {
            tokio::select! {
                // 按刷新间隔采集一次系统指标
                _ = interval.tick() => {
                    let overview = collector::collect_system_overview();
                    // CPU 大核调度：判据用等效繁忙核心数（与在线核数无关），
                    // 温度只取 CPU/集群相关传感器，避免被电池、modem 等区域干扰
                    let max_cpu_temp = overview
                        .thermal
                        .iter()
                        .filter(|z| z.name.contains("cpu") || z.name.contains("cluster"))
                        .map(|z| z.temp_celsius)
                        .fold(0.0_f64, f64::max);
                    collector::cpu_power::auto_schedule(overview.cpu.busy_cores, max_cpu_temp);
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
            // Web 端改了刷新间隔就重建定时器；新定时器首个 tick 立即到期，改动即刻生效
            let want = refresh_bg.load(Ordering::Relaxed);
            if want != cur_secs {
                cur_secs = want;
                interval = tokio::time::interval(std::time::Duration::from_secs(want));
            }
        }
    });

    // ── 可选物理屏横屏仪表（DRM/KMS 直绘 + 软件旋转）──
    //
    // 初始化失败必须让进程退出，启动器据此回退到 kmscon/ASCII TUI；
    // 否则会出现「API 活着但屏幕空白」的静默失败。
    if enable_screen {
        let rotate_arg = args
            .iter()
            .position(|a| a == "--rotate")
            .and_then(|i| args.get(i + 1))
            .map(|s| s.as_str());
        let rot = match rotate_arg {
            Some("270") => screen::Rotation::Rot270,
            _ => screen::Rotation::Rot90,
        };
        match screen::open(rot) {
            Ok(scr) => {
            let rx_screen = state.latest.clone();
            let db_screen = db.clone();
            let refresh_screen = refresh_secs.clone();
            std::thread::spawn(move || {
                if let Err(e) = screen::run(scr, rx_screen, Some(db_screen), refresh_screen) {
                        tracing::error!("screen: 渲染循环退出: {}", e);
                    }
                });
            }
            Err(e) => {
                tracing::error!("screen: 初始化失败: {} — 退出以便回退 kmscon/ASCII", e);
                std::process::exit(3);
            }
        }
    }

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
        .route("/system/refresh", get(api::system::get_refresh).post(api::system::set_refresh))
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

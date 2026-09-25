//! 天气数据（Open-Meteo 免费接口，无需 API key）。
//!
//! 数据源：`https://api.open-meteo.com/v1/forecast`
//! - 免费、无需注册，支持 `timezone=Asia/Shanghai` 直接返回本地时刻的逐时/当日值
//! - 同时返回当日 sunrise/sunset，比本地天文推算更权威（推算值与之差约 2~3 分钟）
//!
//! 取数方式用 `curl` 子进程而不是 HTTP 库：项目里的 `ureq` 是
//! `default-features = false`（不带 TLS），为一个 15 分钟一次的请求去引入
//! rustls 依赖树不划算；curl 在 postmarketOS 自带且已在服务 PATH 上。
//!
//! 刷新策略：独立线程每 15 分钟一次，失败保留上次成功数据并把错误带进 UI
//! （屏幕上看得到「离线」而不是静默显示旧值）。

use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

/// 西安坐标（与 `clock.rs` 的日出日落推算保持一致）。
const LAT: f64 = 34.3416;
const LON: f64 = 108.9398;
const REFRESH_SECS: u64 = 900;

/// 一次天气快照。
#[derive(Clone)]
pub struct Weather {
    /// 最近一次请求是否成功
    pub ok: bool,
    pub error: Option<String>,
    /// 最近一次成功刷新的时间
    pub updated: Option<Instant>,
    pub temp_c: f64,
    pub feels_c: f64,
    pub humidity: f64,
    /// WMO 天气代码
    pub code: u32,
    /// 风速 m/s
    pub wind_ms: f64,
    /// 风向（度，气象学约定：风的来向）
    pub wind_dir: f64,
    pub precip_mm: f64,
    pub t_max: f64,
    pub t_min: f64,
    pub precip_prob: f64,
    /// 当日日出 `HH:MM`（接口返回，权威值）
    pub sunrise: Option<String>,
    pub sunset: Option<String>,
    /// 次日日出/日落。取 2 天是为了让「明日昼长变化」与今日同源，
    /// 否则「接口今日 vs 推算明日」会把两个口径的系统偏差算进差值里。
    pub sunrise2: Option<String>,
    pub sunset2: Option<String>,
}

impl Default for Weather {
    fn default() -> Self {
        Self {
            ok: false,
            error: None,
            updated: None,
            temp_c: 0.0,
            feels_c: 0.0,
            humidity: 0.0,
            code: 0,
            wind_ms: 0.0,
            wind_dir: 0.0,
            precip_mm: 0.0,
            t_max: 0.0,
            t_min: 0.0,
            precip_prob: 0.0,
            sunrise: None,
            sunset: None,
            sunrise2: None,
            sunset2: None,
        }
    }
}

/// WMO 天气代码 → 中文描述。
pub fn code_desc(c: u32) -> &'static str {
    match c {
        0 => "晴",
        1 => "少云",
        2 => "多云",
        3 => "阴",
        45 | 48 => "雾",
        51 => "小毛毛雨",
        53 => "毛毛雨",
        55 => "大毛毛雨",
        56 | 57 => "冻毛毛雨",
        61 => "小雨",
        63 => "中雨",
        65 => "大雨",
        66 | 67 => "冻雨",
        71 => "小雪",
        73 => "中雪",
        75 => "大雪",
        77 => "雪粒",
        80 => "小阵雨",
        81 => "中阵雨",
        82 => "大阵雨",
        85 => "小阵雪",
        86 => "大阵雪",
        95 => "雷阵雨",
        96 | 99 => "雷阵雨伴冰雹",
        _ => "未知",
    }
}

/// 风向度数 → 八方位中文（气象学约定：风的来向）。
pub fn wind_dir_cn(deg: f64) -> &'static str {
    const DIRS: [&str; 8] = ["北", "东北", "东", "东南", "南", "西南", "西", "西北"];
    let idx = (((deg % 360.0) + 360.0) % 360.0 / 45.0).round() as usize % 8;
    DIRS[idx]
}

/// 是否属于降水天气（用于卡片着色）。
pub fn is_wet(c: u32) -> bool {
    matches!(c, 51..=67 | 71..=77 | 80..=82 | 85 | 86 | 95..=99)
}

type Shared = Arc<Mutex<Weather>>;
static WEATHER: OnceLock<Shared> = OnceLock::new();

/// 取共享快照（未启动采集时返回空快照）。
pub fn shared() -> Shared {
    WEATHER.get_or_init(|| Arc::new(Mutex::new(Weather::default()))).clone()
}

/// 启动后台刷新线程（重复调用只启动一次）。
pub fn start_feed() -> Shared {
    let shared = shared();
    // swap 返回旧值：已经是 true 说明线程已起过
    if STARTED.swap(true, std::sync::atomic::Ordering::SeqCst) {
        return shared;
    }
    let s = shared.clone();
    std::thread::Builder::new()
        .name("weather-feed".into())
        .spawn(move || loop {
            let w = fetch();
            if let Ok(mut g) = s.lock() {
                let prev_ok = g.ok;
                let prev_updated = g.updated;
                match w {
                    Ok(mut fresh) => {
                        fresh.ok = true;
                        fresh.updated = Some(Instant::now());
                        // 成功时清掉上次的错误
                        *g = fresh;
                    }
                    Err(e) => {
                        tracing::warn!("weather: 刷新失败: {e}");
                        if prev_ok {
                            // 保留上次成功的数据，只标错误
                            g.ok = false;
                            g.error = Some(e);
                        } else {
                            let u = prev_updated;
                            *g = Weather { ok: false, error: Some(e), updated: u, ..Weather::default() };
                        }
                    }
                }
            }
            std::thread::sleep(Duration::from_secs(REFRESH_SECS));
        })
        .ok();
    shared
}

static STARTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn url() -> String {
    format!(
        "https://api.open-meteo.com/v1/forecast?latitude={LAT}&longitude={LON}\
         &current=temperature_2m,relative_humidity_2m,apparent_temperature,weather_code,\
wind_speed_10m,wind_direction_10m,precipitation\
         &daily=temperature_2m_max,temperature_2m_min,precipitation_probability_max,sunrise,sunset\
         &wind_speed_unit=ms&timezone=Asia%2FShanghai&forecast_days=2"
    )
}

fn fetch() -> Result<Weather, String> {
    // 直连与走代理各试一次：链路抖动时（曾长期出现 curl exit 35 TLS
    // unexpected eof）单次请求就放弃会让天气卡整刻钟显示「离线」。
    let url = url();
    let attempts: [Vec<&str>; 2] = [
        vec!["-s", "--max-time", "20", "-H", "Accept: application/json", &url],
        vec!["-s", "--max-time", "20", "--noproxy", "*", "-H", "Accept: application/json", &url],
    ];
    let mut last_err = String::new();
    for args in attempts {
        match std::process::Command::new("curl").args(&args).output() {
            Ok(out) if out.status.success() => {
                return parse_body(&String::from_utf8_lossy(&out.stdout));
            }
            Ok(out) => {
                last_err = format!("curl 退出码 {}", out.status.code().unwrap_or(-1));
            }
            Err(e) => last_err = format!("无法执行 curl: {e}"),
        }
    }
    Err(last_err)
}

/// 解析 Open-Meteo 响应。抽成独立函数是为了能在测试里直接喂样本 JSON——
/// 这里最容易踩的坑是 `daily` 下的字段是**数组**、`current` 下是标量，
/// 混用 `as_f64()` 会静默取到 0。
fn parse_body(body: &str) -> Result<Weather, String> {
    let v: serde_json::Value = serde_json::from_str(body).map_err(|e| format!("JSON 解析失败: {e}"))?;
    if let Some(err) = v.get("reason").and_then(|r| r.as_str()) {
        return Err(format!("接口返回错误: {err}"));
    }
    let cur = v.get("current").ok_or("响应缺少 current")?;
    let daily = v.get("daily").ok_or("响应缺少 daily")?;
    let num = |o: &serde_json::Value, k: &str| o.get(k).and_then(|x| x.as_f64()).unwrap_or(0.0);
    let nth = |o: &serde_json::Value, k: &str, i: usize| -> Option<String> {
        o.get(k)?.as_array()?.get(i)?.as_str().map(|s| s.to_string())
    };
    let first = |o: &serde_json::Value, k: &str| nth(o, k, 0);
    let first_num = |o: &serde_json::Value, k: &str| {
        o.get(k)
            .and_then(|x| x.as_array())
            .and_then(|a| a.first())
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0)
    };
    // sunrise/sunset 形如 "2026-09-15T06:26"，只取时间部分
    let hhmm = |s: Option<String>| -> Option<String> {
        s.and_then(|s| s.split('T').nth(1).map(|t| t.chars().take(5).collect()))
    };
    Ok(Weather {
        ok: true,
        error: None,
        updated: Some(Instant::now()),
        temp_c: num(cur, "temperature_2m"),
        feels_c: num(cur, "apparent_temperature"),
        humidity: num(cur, "relative_humidity_2m"),
        code: num(cur, "weather_code") as u32,
        wind_ms: num(cur, "wind_speed_10m"),
        wind_dir: num(cur, "wind_direction_10m"),
        precip_mm: num(cur, "precipitation"),
        t_max: first_num(daily, "temperature_2m_max"),
        t_min: first_num(daily, "temperature_2m_min"),
        precip_prob: first_num(daily, "precipitation_probability_max"),
        sunrise: hhmm(first(daily, "sunrise")),
        sunset: hhmm(first(daily, "sunset")),
        sunrise2: hhmm(nth(daily, "sunrise", 1)),
        sunset2: hhmm(nth(daily, "sunset", 1)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 回归测试：`daily` 字段是数组、`current` 是标量。
    /// 用 as_f64() 解数组会静默得到 0（曾表现为「今日 0~0°C」）。
    #[test]
    fn daily字段必须按数组解析() {
        let body = r#"{
          "current":{"temperature_2m":21.9,"relative_humidity_2m":52,"apparent_temperature":20.0,
                     "weather_code":3,"wind_speed_10m":4.5,"wind_direction_10m":144.0,"precipitation":0.0},
          "daily":{"temperature_2m_max":[27.4,26.1],"temperature_2m_min":[17.2,16.8],
                   "precipitation_probability_max":[0,10],
                   "sunrise":["2026-09-15T06:26","2026-09-16T06:27"],
                   "sunset":["2026-09-15T18:51","2026-09-16T18:49"]}
        }"#;
        let w = parse_body(body).expect("解析失败");
        assert_eq!(w.t_max, 27.4, "daily 数组首元素没取到");
        assert_eq!(w.t_min, 17.2);
        assert_eq!(w.precip_prob, 0.0);
        assert_eq!(w.temp_c, 21.9);
        assert_eq!(w.sunrise.as_deref(), Some("06:26"));
        assert_eq!(w.sunset.as_deref(), Some("18:51"));
        assert_eq!(w.sunrise2.as_deref(), Some("06:27"), "次日日出没取到");
        assert_eq!(w.sunset2.as_deref(), Some("18:49"));
    }

    #[test]
    fn 天气代码与风向描述() {
        assert_eq!(code_desc(0), "晴");
        assert_eq!(code_desc(3), "阴");
        assert_eq!(code_desc(95), "雷阵雨");
        assert_eq!(code_desc(12345), "未知");
        assert_eq!(wind_dir_cn(0.0), "北");
        assert_eq!(wind_dir_cn(90.0), "东");
        assert_eq!(wind_dir_cn(180.0), "南");
        assert_eq!(wind_dir_cn(225.0), "西南");
        assert_eq!(wind_dir_cn(359.0), "北");
        assert_eq!(wind_dir_cn(-45.0), "西北");
        assert!(is_wet(61) && is_wet(95));
        assert!(!is_wet(0) && !is_wet(3));
    }
}

//! 电池指标采集
//!
//! 读取高通平台 `/sys/class/power_supply/qcom-battery/` 下的电源管理节点。
//! 并根据设计容量与当前电流估算剩余/充满时间。

use super::BatteryInfo;
use std::fs;
use std::path::Path;

const BATTERY_SUPPLY: &str = "qcom-battery";
/// 判定为充电的最小电流（μA）
const CHARGE_CURRENT_UA: f64 = 100_000.0;
/// 判定为放电的最小电流（μA，负值）
const DISCHARGE_CURRENT_UA: f64 = -30_000.0;
/// 判定电池健康衰减：实际上限低于此值。
/// 学习逻辑与 `is_degraded` 共用这一个阈值，避免两处判断不一致
/// （曾出现「UI 说老化、学习器说健康」的矛盾）。
const DEGRADED_MAX_THRESHOLD: u8 = 97;
/// 连续多少次充电都充不满才算真衰减
const MIN_DEGRADE_STREAK: u8 = 3;
const EFFECTIVE_MAX_FILE: &str = "battery_effective_max.txt";
/// 当前充电会话的最高容量（正在上升中）
const SESSION_PEAK_FILE: &str = "battery_session_peak.txt";
/// 连续充不满的次数（只有拔掉充电器才算一次）
const LOW_STREAK_FILE: &str = "battery_low_streak.txt";
/// 本次充电会话里，充电器是否把充电走完了（内核报 Full，或电流收尾到阈值以下）。
/// 这是区分「电池真的充不满」和「用户中途拔了」的唯一依据。
const SESSION_DONE_FILE: &str = "battery_session_done.txt";

/// 读取指定 power_supply 节点的字符串值。
fn read_supply(supply: &str, field: &str) -> String {
    let path = format!("/sys/class/power_supply/{supply}/{field}");
    fs::read_to_string(&path)
        .unwrap_or_default()
        .trim()
        .to_string()
}

/// 读取 qcom-battery 节点的字符串值。
fn read_power_supply(field: &str) -> String {
    read_supply(BATTERY_SUPPLY, field)
}

/// 读取 qcom-battery 节点的浮点数值。
fn read_power_supply_f64(field: &str) -> f64 {
    read_power_supply(field).parse::<f64>().unwrap_or(0.0)
}

/// 检测 USB / AC / 无线充电器是否在线。
fn is_external_power_online() -> bool {
    let dir = Path::new("/sys/class/power_supply");
    let Ok(entries) = fs::read_dir(dir) else {
        return false;
    };

    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name == BATTERY_SUPPLY || name == "battery" {
            continue;
        }

        let supply_type = read_supply(&name, "type");
        if !matches!(supply_type.as_str(), "USB" | "Mains" | "Wireless") {
            continue;
        }

        if read_supply(&name, "online") == "1" {
            return true;
        }
    }

    false
}

/// 结合 sysfs status、电流方向与外部电源，修正充电状态。
fn normalize_status(raw_status: &str, current_ua: f64, usb_online: bool) -> String {
    if !usb_online {
        return if raw_status == "Full" {
            "Full".into()
        } else {
            "Discharging".into()
        };
    }

    match raw_status {
        "Full" => "Full".into(),
        "Not charging" => "Not charging".into(),
        "Charging" => {
            if current_ua < CHARGE_CURRENT_UA {
                "Not charging".into()
            } else {
                "Charging".into()
            }
        }
        "Discharging" => "Discharging".into(),
        _ => {
            if current_ua > CHARGE_CURRENT_UA {
                "Charging".into()
            } else if current_ua < DISCHARGE_CURRENT_UA {
                "Discharging".into()
            } else {
                "Not charging".into()
            }
        }
    }
}

fn load_u8(filename: &str) -> u8 {
    fs::read_to_string(filename)
        .ok()
        .and_then(|s| s.trim().parse::<u8>().ok())
        .unwrap_or(0)
}

fn save_u8(filename: &str, value: u8) {
    let _ = fs::write(filename, format!("{}\n", value));
}

/// 文件值 → 实际上限的纯转换，单独抽出来是为了能直接单测。
///
/// **缺失/损坏（0）必须回落 100（不缩放）**。旧写法是
/// `load_u8(f).clamp(50,100).max(50)`，文件缺失时得到 50 ——
/// 于是 `display_capacity_pct` 把电量**翻倍**显示（48% 显示成 96%），
/// 还会被 `is_degraded` 误判成老化电池。缩放只在「确实学到过更低的上限」时
/// 才允许发生，默认状态必须是透明的。
fn effective_max_from_file(raw: u8) -> u8 {
    if raw == 0 {
        100
    } else {
        raw.clamp(50, 100)
    }
}

fn load_effective_max_pct() -> u8 {
    if let Ok(env) = std::env::var("BATTERY_EFFECTIVE_MAX_PCT") {
        if let Ok(v) = env.parse::<u8>() {
            return v.clamp(50, 100);
        }
    }
    effective_max_from_file(load_u8(EFFECTIVE_MAX_FILE))
}

fn save_effective_max_pct(value: u8) {
    save_u8(EFFECTIVE_MAX_FILE, value.clamp(50, 100));
}

/// 根据充电截止行为学习实际上限 SOC（老化电池可能到不了 100%）。
///
/// ## 策略
///
/// 1. **立即上调** —— 如果 raw capacity 大于当前上限，直接更新。
///    硬件报告了更高的电量，没有理由不信任。
///
/// 2. **追踪充电峰值** —— 每次插着电时，记录本次充电达到的最高容量。
///
/// 3. **降级需确认** —— 只有在拔掉充电器时，如果本次峰值明显低于当前上限，
///    才记一次「低峰值」。连续 3 次低峰值后才真正下调 effective_max。
///    中途只要有一次充到接近上限，就把低峰值计数清零。
///
/// 这样既不会因为单次充电异常（温度保护、中间暂停）就永久降级，
/// 也不会因为电池偶尔恢复就错过真正的衰减。
#[derive(Debug, PartialEq)]
enum SessionOutcome {
    /// 充到接近实际上限 → 电池健康，清零「充不满」计数
    ReachedFull,
    /// 充电器走完了充电，容量却明显低于上限 → 衰减证据
    Degraded,
    /// 中途拔掉充电器 → 什么都说明不了（低峰值可能只是用户拔早了）
    Inconclusive,
}

/// 判断本次会话的结论。
///
/// **只凭「充电峰值低」判衰减是错的**：用户没充满就拔掉同样是低峰值。
/// 本机实测到的典型序列就是「没电→插上充到 4x%→拔掉用」，
/// 连来三次就会把 effective_max 下调到 4x%，之后电量按比例放大显示
/// （真实 45% 显示成 100%），而且再也回不去。区分依据只有一个：
/// **充电器有没有把充电走完**（内核报 Full，或电流收尾到阈值以下）。
fn session_outcome(peak: u8, max_pct: u8, charge_done: bool) -> SessionOutcome {
    if peak.saturating_add(2) >= max_pct {
        SessionOutcome::ReachedFull
    } else if charge_done {
        SessionOutcome::Degraded
    } else {
        SessionOutcome::Inconclusive
    }
}

/// 学习「实际上限 SOC」并在插拔电时给出结论（策略见上方文档）。
fn learn_effective_max_pct(capacity: u8, usb_online: bool, charge_done: bool) -> u8 {
    let max_pct = load_effective_max_pct();

    // ── [规则 1] raw capacity 超过当前上限 → 立即上调 ──
    if capacity > max_pct {
        save_effective_max_pct(capacity);
        save_u8(SESSION_PEAK_FILE, 0);
        save_u8(LOW_STREAK_FILE, 0);
        return capacity;
    }

    // ── [规则 2] 不插电时 → 判断本次充电会话是否结束 ──
    if !usb_online {
        let peak = load_u8(SESSION_PEAK_FILE);

        // 正在放电，还没充过电 → 无会话
        if peak == 0 {
            return max_pct;
        }

        // 充电会话结束（拔充电器时 peak=0 已被上面的 guard 拦截，不会进来）
        let done = load_u8(SESSION_DONE_FILE) == 1;
        match session_outcome(peak, max_pct, done) {
            SessionOutcome::ReachedFull => {
                let streak = load_u8(LOW_STREAK_FILE);
                if streak > 0 {
                    println!("[battery] good charge cycle (peak={}%), cleared low streak", peak);
                }
                save_u8(LOW_STREAK_FILE, 0);
            }
            SessionOutcome::Degraded => {
                let streak = load_u8(LOW_STREAK_FILE) + 1;
                save_u8(LOW_STREAK_FILE, streak);
                if streak >= MIN_DEGRADE_STREAK {
                    // 连续 N 次「充电器走完却只到这么点」→ 真衰减
                    println!(
                        "[battery] degradation confirmed: charger finished at {}% {} times, lowering max {}% → {}%",
                        peak, streak, max_pct, peak,
                    );
                    save_effective_max_pct(peak);
                } else {
                    println!(
                        "[battery] low peak {}% with charger finished (streak {}/{})",
                        peak, streak, MIN_DEGRADE_STREAK,
                    );
                }
            }
            SessionOutcome::Inconclusive => {
                // 用户中途拔掉的，不计入也不清零：它既不能证明衰减，
                // 也不该顶掉之前累积的衰减证据
                println!("[battery] session ended early at {}%, not counted", peak);
            }
        }

        // 重置会话状态
        save_u8(SESSION_PEAK_FILE, 0);
        save_u8(SESSION_DONE_FILE, 0);
        return max_pct;
    }

    // ── [规则 3] 插着电：追踪会话峰值，并记录充电器是否把充电走完 ──
    let peak = load_u8(SESSION_PEAK_FILE);
    if capacity > peak {
        save_u8(SESSION_PEAK_FILE, capacity);
    }
    if charge_done {
        save_u8(SESSION_DONE_FILE, 1);
    }

    max_pct
}

// ── 实际容量学习 ──
//
// 设计容量（`charge_full_design`）是出厂值，锂电老化后会明显低于它。
// 用它算「还能用多久 / 还要充多久」会**系统性偏乐观**：本机实测设计 3200mAh、
// 实际约 2200mAh，于是电量 53% 时报「可用 5.6h」，而同样用法实测只能撑约 3.9h。
// 解法与 effective_max 同源：让程序自己从放电过程量出来。
//
// 依据是电荷守恒：放出的电荷 ÷ 电量下降幅度 × 100 = 满容量。
// 三次独立放完电实测得到 2160 / 2545 / 2153 mAh（±10% 内一致），
// 说明这个量是可靠的，不是拍脑袋。

/// 观测到的实际满容量（mAh）；0 = 还没量出来
const CAPACITY_MAH_FILE: &str = "battery_capacity_mah.txt";
/// 放电会话累计状态："起始SOC 累计mAh 上次时间戳 本段是否已测量"
const DISCHARGE_FILE: &str = "battery_discharge.txt";
/// 间隔超过这么久视为新的放电会话（中途系统睡着/充过电，积分就不连续了）
const DISCHARGE_GAP_SECS: i64 = 600;
/// 至少要掉这么多百分点（跨度太小，噪声占主导）
const MIN_SPAN_PCT: u32 = 50;
/// 还必须**放到这么低**才算量到低端。电量计的 SOC 与实际电荷非线性：
/// **0-9% 这一档握着约 20% 的电荷**（三次完整放电实测：该档 126 mAh/1%，
/// 其余档只有 45~61 mAh/1%），所以只测上半段会把容量系统性算小
/// —— 实测 99%→47% 只推得 1616 mAh，而真值约 2080。
const LOW_END_PCT: u8 = 15;
/// 容量估值的合理区间（mAh），明显越界的一律丢弃
const CAPACITY_SANITY: (f64, f64) = (800.0, 6000.0);
/// 已有估值时新观测的融合权重（单次积分噪声不小，整段替换会让显示跳变）
const CAPACITY_BLEND: f64 = 0.3;

fn load_f64(filename: &str) -> f64 {
    fs::read_to_string(filename)
        .ok()
        .and_then(|s| s.trim().parse::<f64>().ok())
        .unwrap_or(0.0)
}

fn save_f64(filename: &str, value: f64) {
    let _ = fs::write(filename, format!("{value:.1}\n"));
}

/// 这段放电能不能拿来定容量：跨度够大，**且必须量到低端**（见 LOW_END_PCT 的说明）。
fn discharge_is_measurable(span_pct: u32, end_soc: u8) -> bool {
    span_pct >= MIN_SPAN_PCT && end_soc <= LOW_END_PCT
}

/// 由「放出的电荷 / 电量下降幅度」推算容量，越界返回 None（纯函数，便于单测）。
fn capacity_from_discharge(accum_mah: f64, span_pct: u32) -> Option<f64> {
    if span_pct == 0 || accum_mah <= 0.0 {
        return None;
    }
    let est = accum_mah / span_pct as f64 * 100.0;
    (est >= CAPACITY_SANITY.0 && est <= CAPACITY_SANITY.1).then_some(est)
}

/// 新旧容量融合：首次直接采纳，之后小幅修正。
/// 一条放电段会给出很多次估值，全量采纳等于让最近一次负载波动决定显示值。
fn blend_capacity(old: f64, est: f64) -> f64 {
    if old <= 0.0 {
        est
    } else {
        old * (1.0 - CAPACITY_BLEND) + est * CAPACITY_BLEND
    }
}

fn load_discharge() -> (u8, f64, i64, bool) {
    let s = fs::read_to_string(DISCHARGE_FILE).unwrap_or_default();
    let f: Vec<&str> = s.split_whitespace().collect();
    let g = |i: usize| f.get(i).and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0);
    (
        g(0) as u8,
        g(1),
        f.get(2).and_then(|v| v.parse::<i64>().ok()).unwrap_or(0),
        f.get(3).map(|v| *v == "1").unwrap_or(false),
    )
}

fn save_discharge(soc0: u8, accum_mah: f64, ts: i64, measured: bool) {
    let _ = fs::write(
        DISCHARGE_FILE,
        format!("{soc0} {accum_mah:.1} {ts} {}\n", if measured { 1 } else { 0 }),
    );
}

/// 放电时累积电荷，跨度够了就更新实际容量估计。只在真正放电且电流够大时积分。
fn learn_capacity(capacity: u8, current_ua: f64, status: &str) {
    if status != "Discharging" || current_ua.abs() < 20_000.0 {
        return;
    }
    let now = chrono::Utc::now().timestamp();
    let (soc0, accum, last_ts, measured) = load_discharge();

    // 新会话：没记录过、间隔太久（系统睡过/充过电）、或电量反而回升了
    if last_ts == 0 || now - last_ts > DISCHARGE_GAP_SECS || capacity > soc0 {
        save_discharge(capacity, 0.0, now, false);
        return;
    }

    let dt = (now - last_ts).clamp(0, DISCHARGE_GAP_SECS) as f64;
    let accum = accum + current_ua.abs() * dt / 3600.0 / 1000.0; // μA·s → mAh
    let span = soc0.saturating_sub(capacity) as u32;

    if !measured && discharge_is_measurable(span, capacity) {
        if let Some(est) = capacity_from_discharge(accum, span) {
            save_f64(CAPACITY_MAH_FILE, blend_capacity(load_f64(CAPACITY_MAH_FILE), est));
            tracing::info!(
                "电池实际容量估计 {est:.0} mAh（从 {soc0}% 掉到 {capacity}%，累计放出 {accum:.0} mAh）"
            );
            // 本段已量过：重新起一段，避免同一段被反复测量反复融合
            save_discharge(capacity, 0.0, now, true);
            return;
        }
    }
    save_discharge(soc0, accum, now, measured);
}

fn display_capacity_pct(capacity: u8, effective_max_pct: u8) -> u8 {
    if effective_max_pct >= 100 || effective_max_pct == 0 {
        return capacity;
    }
    ((capacity as f64 / effective_max_pct as f64) * 100.0)
        .clamp(0.0, 100.0)
        .round() as u8
}

fn at_charge_limit(capacity: u8, effective_max_pct: u8, usb_online: bool, current_ua: f64) -> bool {
    usb_online
        && capacity >= effective_max_pct
        && current_ua.abs() < CHARGE_CURRENT_UA
}

/// 估算剩余 / 充满时间（分钟）。
///
/// **单位必须是 sysfs 原样**：容量 μAh、电流 μA，相除就是小时。
/// 混用 mAh 与 μA 会得到 1000 倍误差 —— 这个 bug 真的发生过
/// （「还要充 20 分钟」被算成 0），而且因为计算写在 collect() 里、不可单测，
/// 只能靠在真机上肉眼发现。所以抽成纯函数并用真实量级的数值测。
///
/// 约定：放电返回正数（还能用多少分钟），充电返回负数（还需多少分钟充满），
/// 无法估算返回 0。
fn estimate_time_min(
    capacity: u8,
    status: &str,
    current_ua: f64,
    usable_capacity_uah: f64,
    effective_max_pct: u8,
    at_limit: bool,
) -> i64 {
    let i = current_ua.abs();
    if at_limit || i < 1000.0 || usable_capacity_uah <= 0.0 {
        return 0;
    }
    match status {
        "Discharging" => {
            let charge_now = usable_capacity_uah * (capacity as f64 / 100.0);
            ((charge_now / i) * 60.0).max(0.0) as i64
        }
        "Charging" => {
            let headroom = effective_max_pct.saturating_sub(capacity) as f64;
            let charge_needed = usable_capacity_uah * (headroom / 100.0);
            -((charge_needed / i) * 60.0).max(0.0) as i64
        }
        _ => 0,
    }
}

/// 采集电池容量、状态、电压、电流、温度及预估时间。
pub fn collect() -> BatteryInfo {
    let capacity: u8 = read_power_supply("capacity").parse().unwrap_or(0);
    let raw_status = read_power_supply("status");
    let voltage = read_power_supply_f64("voltage_now") / 1_000_000.0; // μV → V
    let current_ua = read_power_supply_f64("current_now");
    let current = current_ua / 1_000_000.0; // μA → A
    let temp = read_power_supply_f64("temp") / 10.0; // 0.1°C → °C

    let usb_online = is_external_power_online();
    let mut status = normalize_status(&raw_status, current_ua, usb_online);

    // 「充电器已把充电走完」：内核报 Full，或电流已收尾且容量贴近当前上限。
    // 注意用**学习前**的上限来判断，否则与 learn 内部读到的上限不一致
    let max_before = load_effective_max_pct();
    let charge_done = usb_online
        && (raw_status == "Full"
            || (current_ua.abs() < CHARGE_CURRENT_UA && capacity.saturating_add(2) >= max_before));

    let effective_max_pct = learn_effective_max_pct(capacity, usb_online, charge_done);
    let is_degraded = effective_max_pct < DEGRADED_MAX_THRESHOLD;
    let at_limit = at_charge_limit(capacity, effective_max_pct, usb_online, current_ua);
    let display_capacity_pct = display_capacity_pct(capacity, effective_max_pct);

    if at_limit && usb_online {
        status = "Full".into();
    }

    // 放电时顺便量实际容量（充电中不量：积分不连续）
    learn_capacity(capacity, current_ua, &status);

    // charge_full_design: μAh, current_now: μA (= μAh/h)
    let charge_full_design = read_power_supply_f64("charge_full_design");
    let current_magnitude_ua = current_ua.abs();

    // 时间估算用**实际容量**；还没量出来才退回设计容量（那一步会偏乐观）。
    // 单位统一到 sysfs 原样（μAh / μA）：**mAh 与 μA 混用会差 1000 倍**，
    // 曾经因此把「还要充 20 分钟」算成 0。
    let learned_capacity_mah = load_f64(CAPACITY_MAH_FILE);
    let usable_uah = if learned_capacity_mah > 0.0 {
        learned_capacity_mah * 1000.0
    } else {
        charge_full_design
    };
    let health_percent = if learned_capacity_mah > 0.0 && charge_full_design > 0.0 {
        let design_mah = charge_full_design / 1000.0;
        (learned_capacity_mah / design_mah * 100.0).clamp(1.0, 150.0)
    } else {
        0.0
    };

    let time_left_min = estimate_time_min(
        capacity,
        &status,
        current_ua,
        usable_uah,
        effective_max_pct,
        at_limit,
    );

    BatteryInfo {
        capacity,
        status,
        voltage_v: voltage,
        current_ma: current * 1000.0,
        power_w: voltage * current.abs(),
        temp_celsius: temp,
        time_left_min,
        effective_max_pct,
        display_capacity_pct,
        is_degraded,
        at_charge_limit: at_limit,
        capacity_mah: learned_capacity_mah,
        health_percent,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unplugged_charging_status_becomes_discharging() {
        assert_eq!(
            normalize_status("Charging", 245_000.0, false),
            "Discharging"
        );
    }

    #[test]
    fn plugged_low_current_is_not_charging() {
        assert_eq!(
            normalize_status("Charging", 50_000.0, true),
            "Not charging"
        );
    }

    #[test]
    fn plugged_high_current_is_charging() {
        assert_eq!(
            normalize_status("Charging", 500_000.0, true),
            "Charging"
        );
    }

    #[test]
    fn unplugged_full_stays_full() {
        assert_eq!(normalize_status("Full", 0.0, false), "Full");
    }

    #[test]
    fn display_capacity_scales_to_effective_max() {
        assert_eq!(display_capacity_pct(97, 97), 100);
        assert_eq!(display_capacity_pct(85, 97), 88);
    }

    /// 回归：文件缺失/损坏时**必须不缩放**。
    /// 旧默认值 50 会把电量翻倍显示（48% → 96%）并误报老化。
    #[test]
    fn 上限文件缺失时回落为不缩放() {
        assert_eq!(effective_max_from_file(0), 100, "缺失/损坏必须回落不缩放");
        assert_eq!(effective_max_from_file(99), 99);
        assert_eq!(effective_max_from_file(50), 50);
        assert_eq!(effective_max_from_file(200), 100, "越界要夹回合法区间");
        // 语义对照：不缩放时显示值 == 真实值；若上限是 50 就会翻倍
        assert_eq!(display_capacity_pct(48, effective_max_from_file(0)), 48);
        assert_eq!(display_capacity_pct(48, 50), 96, "这就是旧默认值造成的翻倍");
    }

    /// 回归：中途拔掉充电器**不能**算作衰减证据。
    /// 只凭「峰值低」判断的话，用户没充满就拔掉三次就会把上限下调到低峰值，
    /// 之后电量被按比例放大（真实 45% 显示 100%）且再也回不去。
    #[test]
    fn 中途拔掉不算衰减证据() {
        assert_eq!(session_outcome(50, 99, false), SessionOutcome::Inconclusive,
                   "充电器没走完 → 只是用户拔早了");
        assert_eq!(session_outcome(50, 99, true), SessionOutcome::Degraded,
                   "充电器走完却只到 50% → 真衰减");
        assert_eq!(session_outcome(99, 99, false), SessionOutcome::ReachedFull,
                   "充到接近上限 → 健康（无论是否拔掉）");
        assert_eq!(session_outcome(98, 99, true), SessionOutcome::ReachedFull);
        assert_eq!(session_outcome(97, 99, true), SessionOutcome::ReachedFull, "2% 容差边界");
        assert_eq!(session_outcome(96, 99, true), SessionOutcome::Degraded);
    }

    /// 容量推算：电荷 ÷ 电量跨度 × 100，并挡掉明显越界的估值。
    /// 数据取自数据库里三次真实完整放电（梯形积分）。
    #[test]
    fn 容量由放电积分推算() {
        for (accum, span) in [(1898.0, 98), (2118.0, 98), (2110.0, 98), (1310.0, 61)] {
            let c = capacity_from_discharge(accum, span).expect("应能推算");
            assert!((1800.0..2300.0).contains(&c), "推算 {c:.0} 偏离实测区间");
        }
        assert!(capacity_from_discharge(50.0, 20).is_none(), "电荷太小 → 越界丢弃");
        assert!(capacity_from_discharge(9999.0, 10).is_none(), "明显离谱应丢弃");
        assert!(capacity_from_discharge(500.0, 0).is_none(), "零跨度不能用");
    }

    /// **必须量到低端**才算有效：电量计的 0-9% 档握着约 20% 的电荷，
    /// 只测上半段会系统性算小（实测 99%→47% 只得 1616 mAh，真值约 2080）。
    /// 这是本机数据库里各段的真实起点/终点。
    #[test]
    fn 只测上半段不算有效容量() {
        // 不完整区间：跨度看着够（≥50%）但没量到低端 → 拒绝
        assert!(!discharge_is_measurable(52, 47), "99→47 会把容量算小");
        assert!(!discharge_is_measurable(73, 26), "99→26 同样没量到低端");
        assert!(!discharge_is_measurable(63, 36), "99→36 也不行");
        // 有效：覆盖低端且跨度够
        assert!(discharge_is_measurable(98, 1), "99→1 是完整循环");
        assert!(discharge_is_measurable(61, 1), "62→1 也覆盖了低端，有效");
        // 量到低端但跨度太小 → 噪声占主导，仍拒绝
        assert!(!discharge_is_measurable(20, 1));
    }

    /// 融合：首次采纳，之后小幅修正（别让一次负载波动把显示值带跑）。
    #[test]
    fn 容量融合首次采纳之后小幅修正() {
        assert_eq!(blend_capacity(0.0, 2200.0), 2200.0);
        // 权重 0.3：2200 与 3000 融合后应朝新观测走 30%（=2440），
        // 而不是直接跳到 3000，也不是原地不动
        let b = blend_capacity(2200.0, 3000.0);
        assert!((b - 2440.0).abs() < 1.0, "应朝新观测走 30%，实际 {b:.0}");
        assert!(blend_capacity(2200.0, 2100.0) < 2200.0, "偏低观测也要能往下修正");
    }

    /// 回归：时间估算必须用实际容量，不能用设计容量。
    /// 本机设计 3200mAh 而实际约 2200mAh，用设计值会把「还能用多久」多算约 45%。
    #[test]
    fn 时间估算应基于实际容量() {
        let (design, actual, cur_ua, cap) = (3_200_000.0, 2_200.0, 300_000.0, 50u8);
        let with_design = design * (cap as f64 / 100.0) / cur_ua * 60.0;
        let with_actual = actual * (cap as f64 / 100.0) / cur_ua * 60.0;
        assert!(
            with_design / with_actual > 1.4,
            "设计容量会显著高估（{with_design:.0} vs {with_actual:.0} 分钟），必须用实际容量"
        );
    }

    /// 用**真实 sysfs 量级**的数值测时间估算：这是唯一能钉住单位的地方。
    /// 场景取自实测：80%、+1229mA、实际容量 2160mAh、上限 99% → 约 20 分钟充满。
    #[test]
    fn 时间估算单位必须与sysfs一致() {
        // 充电：2,160,000 μAh 的 19% 由 1,229,000 μA 充 → 约 20 分钟
        let t = estimate_time_min(80, "Charging", 1_229_000.0, 2_160_000.0, 99, false);
        assert!((-25..=-15).contains(&t), "充电估算应在 20 分钟上下，实际 {t}");
        // 放电：2,160,000 μAh 的 53% 由 300,000 μA 放 → 约 229 分钟
        let t = estimate_time_min(53, "Discharging", -300_000.0, 2_160_000.0, 100, false);
        assert!((210..=250).contains(&t), "放电估算应在 229 分钟上下，实际 {t}");
        // 边界：到上限、电流过小、容量未知都应给 0（而不是给个荒谬值）
        assert_eq!(estimate_time_min(99, "Charging", 1_200_000.0, 2_160_000.0, 99, true), 0);
        assert_eq!(estimate_time_min(50, "Discharging", 500.0, 2_160_000.0, 100, false), 0);
        assert_eq!(estimate_time_min(50, "Discharging", -300_000.0, 0.0, 100, false), 0);
        // Not charging / Full 不估时间
        assert_eq!(estimate_time_min(50, "Full", 10_000.0, 2_160_000.0, 99, false), 0);
    }

    #[test]
    fn at_charge_limit_when_plateau_reached() {
        assert!(at_charge_limit(97, 97, true, 20_000.0));
        assert!(!at_charge_limit(96, 97, true, 20_000.0));
    }
}

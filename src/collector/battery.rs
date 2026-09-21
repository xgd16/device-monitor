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
/// 一次充电会话结束时能得出的结论。
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

    // charge_full_design: μAh, current_now: μA (= μAh/h)
    let charge_full_design = read_power_supply_f64("charge_full_design");
    let current_magnitude_ua = current_ua.abs();

    let time_left_min = if at_limit {
        0
    } else if current_magnitude_ua < 1000.0 || charge_full_design <= 0.0 {
        // 电流太小 (< 1mA) 或无设计容量，无法估算
        0
    } else if status == "Discharging" {
        // 剩余 = (capacity% × 设计容量) / 电流 × 60 分钟
        let charge_now = charge_full_design * (capacity as f64 / 100.0);
        ((charge_now / current_magnitude_ua) * 60.0).max(0.0) as i64
    } else if status == "Charging" {
        // 充满 = ((effective_max - capacity)% × 设计容量) / 电流 × 60 分钟
        let headroom = effective_max_pct.saturating_sub(capacity) as f64;
        let charge_needed = charge_full_design * (headroom / 100.0);
        -((charge_needed / current_magnitude_ua) * 60.0).max(0.0) as i64
    } else {
        0
    };

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

    #[test]
    fn at_charge_limit_when_plateau_reached() {
        assert!(at_charge_limit(97, 97, true, 20_000.0));
        assert!(!at_charge_limit(96, 97, true, 20_000.0));
    }
}

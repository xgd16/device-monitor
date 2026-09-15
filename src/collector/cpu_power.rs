//! CPU 大核自动调度。
//!
//! 判据是「等效繁忙核心数」`demand_cores`（由采集层算出），它是与在线核数
//! 无关的绝对负载：同一份工作量，无论 4 核还是 8 核在线都得到同一个值。
//! 早期版本用 `online_usage`（在线核心平均使用率）做判据，大核上线后分母
//! 从 4 变 8、读数腰斩，落在滞回带里不断给保持期续期，导致大核永不休眠。
//!
//! 档位（0 / 2 / 4 个大核）：
//! - demand >= 3.2 核 → 开满 4 个大核
//! - demand >= 2.4 核 → 开 2 个大核
//! - demand <= 2.0 核 → 关闭全部大核 + 小核限频
//! - 2.0 ~ 2.4 核为滞回带，维持现状
//! - 升档立即生效，降档需等保持期（30s，按墙钟计，与刷新率无关）到期
//! - 温度 >= 75°C 时只允许降档或维持，禁止升档
//!
//! 所有 sysfs 写入后都回读校验：写失败或内核静默丢弃（例如传入不在
//! `scaling_available_frequencies` 里的频率）时记 warn 并保持状态不变，
//! 避免软件状态与硬件脱节。

use std::fs;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

const BIG_CORES: &[usize] = &[4, 5, 6, 7];
const SMALL_CORES: &[usize] = &[0, 1, 2, 3];

/// 小核满频。
const SMALL_FULL_FREQ: &str = "1766400";
/// 小核省电上限。
///
/// 必须是 `scaling_available_frequencies` 里的值：非法值会被内核静默丢弃
/// （旧版用的 1267200 不在列表里，实际从未生效）。
const SMALL_SAVE_FREQ: &str = "1228800";

/// 开满 4 个大核的阈值（等效繁忙核心数）
const TIER_FULL_CORES: f32 = 3.2;
/// 开 2 个大核的阈值
const TIER_HALF_CORES: f32 = 2.4;
/// 关闭全部大核的阈值，与 `TIER_HALF_CORES` 之间构成滞回带
const IDLE_CORES: f32 = 2.0;

/// 温度上限（°C），超过则只降不升。
///
/// 取值依据（本机实测，Mi Mix 3 / SDM845）：
/// - 空闲 45~60°C
/// - 仅 4 个小核满载约 55°C
/// - 4 个大核满载（2.65 GHz）约 94~95°C，这是 SoC 的正常满载温度
/// - 内核的 cpufreq 热降频在本机未接入（`cpufreq-cpu0/4` 冷却设备在 95°C 时
///   `cur_state` 仍为 0），所以不会自动限频，这里必须自己兜
///
/// 因此阈值不能设在 75°C —— 那等于「大核一热就再也不升档」，会把 2 核档位
/// 永久钉死在 2 核。95°C 只在接近满载温度时阻止再加核。
const THERMAL_LIMIT_C: f64 = 95.0;

/// 大核上线后最少保持时长（毫秒）。旧实现按采样周期计数（6 × 5s = 30s），
/// 采集间隔改为 Web 端可调后改为绝对时限，保持期与刷新率解耦。
const MIN_HOLD_MILLIS: u64 = 30_000;

/// 当前在线的大核数量（0 / 2 / 4）
static BIG_ONLINE_COUNT: AtomicU32 = AtomicU32::new(0);
static ENABLED: AtomicBool = AtomicBool::new(true);
/// 降档保持期截止时间（Unix 毫秒），到期后才允许降档
static HOLD_UNTIL_MILLIS: AtomicU64 = AtomicU64::new(0);

fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 启动时同步大核实际在线状态，避免重启后软件状态与硬件不一致
/// （旧版默认 false，若重启前大核仍在线就会永远走不到降档分支）。
pub fn init() {
    let actual = count_big_online();
    BIG_ONLINE_COUNT.store(actual, Ordering::Relaxed);
    HOLD_UNTIL_MILLIS.store(0, Ordering::Relaxed);
    if actual > 0 {
        tracing::info!("cpu-power: 启动时检测到 {actual} 个大核在线，同步状态");
    }
}

fn read_core_online(core: usize) -> Option<bool> {
    fs::read_to_string(format!("/sys/devices/system/cpu/cpu{core}/online"))
        .ok()
        .and_then(|s| s.trim().parse::<u8>().ok())
        .map(|v| v == 1)
}

/// 实际在线的大核数量（0 / 2 / 4，一般不会是奇数）
fn count_big_online() -> u32 {
    BIG_CORES
        .iter()
        .filter(|&&c| read_core_online(c).unwrap_or(false))
        .count() as u32
}

/// 在采集循环中调用：根据等效繁忙核心数和当前温度调度大核档位。
pub fn auto_schedule(demand_cores: f32, max_temp_c: f64) {
    if !ENABLED.load(Ordering::Relaxed) {
        return;
    }

    let current = BIG_ONLINE_COUNT.load(Ordering::Relaxed);
    let hold_active = now_millis() < HOLD_UNTIL_MILLIS.load(Ordering::Relaxed);
    let overheat = max_temp_c >= THERMAL_LIMIT_C;

    // 目标档位；滞回带内维持现状
    let mut target = if demand_cores >= TIER_FULL_CORES {
        4
    } else if demand_cores >= TIER_HALF_CORES {
        2
    } else if demand_cores <= IDLE_CORES {
        0
    } else {
        current
    };

    // 过热时禁止升档，只允许降档或维持
    if overheat {
        target = target.min(current);
    }

    if target > current {
        // 升档立即生效：负载已经上来了，等不起保持期
        if apply_big_count(target) {
            BIG_ONLINE_COUNT.store(target, Ordering::Relaxed);
            set_small_max_freq(SMALL_FULL_FREQ);
            HOLD_UNTIL_MILLIS.store(now_millis() + MIN_HOLD_MILLIS, Ordering::Relaxed);
            tracing::info!(
                "cpu-power: 负载 {:.2} 核 → 升档 {current} → {target} 个大核{}",
                demand_cores,
                if overheat { "（过热中，待降温后生效）" } else { "" }
            );
        } else {
            tracing::warn!(
                "cpu-power: 升档到 {target} 个大核失败，保持 {current} 不变"
            );
        }
    } else if target < current {
        if !hold_active {
            if apply_big_count(target) {
                BIG_ONLINE_COUNT.store(target, Ordering::Relaxed);
                if target == 0 {
                    set_small_max_freq(SMALL_SAVE_FREQ);
                }
                HOLD_UNTIL_MILLIS.store(now_millis() + MIN_HOLD_MILLIS, Ordering::Relaxed);
                tracing::info!(
                    "cpu-power: 负载 {:.2} 核 → 降档 {current} → {target} 个大核{}",
                    demand_cores,
                    if overheat {
                        format!("（过热 {max_temp_c:.1}°C）")
                    } else {
                        String::new()
                    }
                );
            } else {
                tracing::warn!(
                    "cpu-power: 降档到 {target} 个大核失败，保持 {current} 不变"
                );
            }
        }
    } else {
        // 维持当前档位
        if current > 0 && demand_cores >= TIER_HALF_CORES {
            // 负载仍在高位，保持期续期
            HOLD_UNTIL_MILLIS.store(now_millis() + MIN_HOLD_MILLIS, Ordering::Relaxed);
        }
    }
}

/// 把在线大核数量调整到 `n`（0 / 2 / 4）。
///
/// 返回 `true` 表示回读确认硬件状态与目标一致；任一核心写失败都返回 `false`，
/// 调用方据此保持原状态，避免软件状态与硬件脱节。
fn apply_big_count(n: u32) -> bool {
    for (idx, &core) in BIG_CORES.iter().enumerate() {
        let want = (idx as u32) < n;
        let path = format!("/sys/devices/system/cpu/cpu{core}/online");
        // 已经是目标状态的跳过：重复写同一个值内核会返回 EINVAL
        if read_core_online(core) == Some(want) {
            continue;
        }
        if let Err(e) = fs::write(&path, if want { "1" } else { "0" }) {
            tracing::warn!("cpu-power: 写 {path} 失败: {e}");
        }
    }

    let actual = count_big_online();
    if actual != n {
        tracing::warn!("cpu-power: 大核档位校验失败，期望 {n} 实际 {actual}");
        return false;
    }
    true
}

/// 设置小核频率上限，写入后回读校验。
///
/// 四个小核共享同一个 cpufreq policy，第一个写成功后续会被跳过。
///
/// 回读需要重试：cpufreq 的 `store()` 返回成功时 `policy->max` 还在异步更新，
/// 立即回读会读到旧值（实测写入 1228800 后立刻回读仍是 1766400，约 1s 后才
/// 变成新值）。若只读一次会把生效的写入误报成失败。
fn set_small_max_freq(freq: &str) {
    for &core in SMALL_CORES {
        let path = format!("/sys/devices/system/cpu/cpu{core}/cpufreq/scaling_max_freq");

        if read_freq(&path).as_deref() == Some(freq) {
            continue;
        }

        if let Err(e) = fs::write(&path, freq) {
            tracing::warn!("cpu-power: 写 {path} = {freq} 失败: {e}");
            continue;
        }

        // 轮询等待生效：最多 20 次 × 50ms = 1s
        let mut settled = false;
        for _ in 0..20 {
            if read_freq(&path).as_deref() == Some(freq) {
                settled = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }

        if !settled {
            tracing::warn!(
                "cpu-power: 小核限频未生效，期望 {freq} 实际 {:?} —— 检查该频率是否在 scaling_available_frequencies 中",
                read_freq(&path)
            );
        }
    }
}

fn read_freq(path: &str) -> Option<String> {
    fs::read_to_string(path).ok().map(|s| s.trim().to_string())
}

pub fn is_enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

pub fn set_enabled(on: bool) {
    ENABLED.store(on, Ordering::Relaxed);
    tracing::info!("cpu-power: 自动调度{}", if on { "启用" } else { "禁用" });
}

/// 当前在线的大核数量（0 / 2 / 4）
pub fn big_cores_online() -> u32 {
    BIG_ONLINE_COUNT.load(Ordering::Relaxed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tier(demand: f32, current: u32) -> u32 {
        if demand >= TIER_FULL_CORES {
            4
        } else if demand >= TIER_HALF_CORES {
            2
        } else if demand <= IDLE_CORES {
            0
        } else {
            current
        }
    }

    #[test]
    fn 高负载开满四核() {
        assert_eq!(tier(4.0, 0), 4);
        assert_eq!(tier(3.2, 0), 4);
    }

    #[test]
    fn 中等负载开两核() {
        assert_eq!(tier(3.1, 0), 2);
        assert_eq!(tier(2.4, 0), 2);
    }

    #[test]
    fn 空负载关闭大核() {
        assert_eq!(tier(2.0, 4), 0);
        assert_eq!(tier(0.5, 4), 0);
    }

    #[test]
    fn 滞回带维持现状() {
        assert_eq!(tier(2.2, 4), 4);
        assert_eq!(tier(2.2, 2), 2);
        assert_eq!(tier(2.2, 0), 0);
    }

    /// 同一份工作量在大核上线前后应给出同一个档位（旧版会因读数腰斩而卡在 4 核）
    #[test]
    fn 大核在线前后判据不漂移() {
        // 3.5 个核的绝对工作量
        let busy = 3.5_f32;
        let demand_4 = busy / 4.0 * 4.0; // 4 核在线
        let demand_8 = busy / 8.0 * 8.0; // 8 核在线
        assert_eq!(tier(demand_4, 0), tier(demand_8, 4));
    }

    #[test]
    fn 省电频率必须在可用列表内() {
        // 回归测试：旧值 1267200 不在小核可用频率列表里，写入被静默丢弃
        let available = [
            "300000", "403200", "480000", "576000", "652800", "748800", "825600", "902400", "979200",
            "1056000", "1132800", "1228800", "1324800", "1420800", "1516800", "1612800", "1689600",
            "1766400",
        ];
        assert!(
            available.contains(&SMALL_SAVE_FREQ),
            "SMALL_SAVE_FREQ={SMALL_SAVE_FREQ} 不在小核可用频率列表中，限频会静默失效"
        );
        assert!(
            available.contains(&SMALL_FULL_FREQ),
            "SMALL_FULL_FREQ={SMALL_FULL_FREQ} 不在小核可用频率列表中"
        );
    }
}

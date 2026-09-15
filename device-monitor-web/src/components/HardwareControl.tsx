import { useState, useEffect, useCallback } from 'react';
import type { ReactNode } from 'react';
import { Button, Chip } from '@heroui/react';
import { Panel } from './Panel';
import {
  fetchHardware,
  setFlashlight,
  setBrightness,
  setScreenPower,
  vibrate,
  vibratePattern,
  vibrateStop,
  clearMemory,
  setStatusLed,
  setCpuStatusLedLink,
  setChargeCurrent,
  setChargeMode,
  setWifiPowerSave,
  setSpeakerVolume,
  setSpeakerMute,
  playSpeakerTest,
} from '../api';
import {
  fmtChargeUa,
  chargeSourceLabel,
  chargeModeLabel,
  isChargePresetSelected,
  fmtMem,
} from './utils';

interface HardwareState {
  flashlight: { white_on: boolean; yellow_on: boolean; max_brightness: number };
  status_led: {
    on: boolean;
    brightness: number;
    max_brightness: number;
    percent: number;
  };
  cpu_status_led_link: {
    enabled: boolean;
    threshold_pct: number;
    link_brightness_pct: number;
    smoothed_cpu_pct: number;
  };
  brightness: { current: number; max: number; percent: number };
  screen_on: boolean;
  vibrating: boolean;
  charging: {
    current_max_ua: number;
    target_current_max_ua: number;
    current_now_ua: number;
    voltage_now_uv: number;
    power_w: number;
    charger_online: boolean;
    usb_type: string;
    charge_source: string;
    wired_max_ua: number;
    wireless_max_ua: number;
    charge_mode: string;
  };
  wifi_power_save: { enabled: boolean; iface: string };
  speaker: {
    available: boolean;
    muted: boolean;
    volume_percent: number;
    sink_name: string;
    backend: string;
  };
}

const BRIGHTNESS_PRESETS = [0, 25, 50, 75, 100];
const SPEAKER_PRESETS = [0, 25, 50, 75, 100];

const CHARGE_PRESETS: { label: string; ua: number; wiredOnly?: boolean }[] = [
  { label: '不限', ua: 0 },
  { label: '500mA', ua: 500_000 },
  { label: '1A', ua: 1_000_000 },
  { label: '1.5A', ua: 1_500_000 },
  { label: '10W', ua: 2_000_000 },
  { label: '2.5A', ua: 2_500_000, wiredOnly: true },
  { label: '3A', ua: 3_000_000, wiredOnly: true },
  { label: '18W', ua: 3_600_000, wiredOnly: true },
];

const VIBE_PRESETS: { name: string; ms: number; strong: number; weak: number }[] = [
  { name: '轻触', ms: 50, strong: 40, weak: 0 },
  { name: '短震', ms: 150, strong: 80, weak: 0 },
  { name: '中震', ms: 400, strong: 80, weak: 0 },
  { name: '长震', ms: 800, strong: 80, weak: 0 },
  { name: '双击', ms: 0, strong: 80, weak: 0 },
  { name: '心跳', ms: 0, strong: 90, weak: 0 },
  { name: 'SOS', ms: 0, strong: 80, weak: 0 },
];

const PATTERNS: Record<string, [number, number, number][]> = {
  双击: [
    [100, 80, 0],
    [80, 0, 0],
    [100, 80, 0],
  ],
  心跳: [
    [100, 90, 0],
    [100, 0, 0],
    [60, 70, 0],
    [500, 0, 0],
  ],
  SOS: [
    [80, 80, 0],
    [80, 0, 0],
    [80, 80, 0],
    [80, 0, 0],
    [80, 80, 0],
    [200, 0, 0],
    [200, 80, 0],
    [200, 0, 0],
    [200, 80, 0],
    [200, 0, 0],
    [200, 80, 0],
    [200, 0, 0],
    [80, 80, 0],
    [80, 0, 0],
    [80, 80, 0],
    [80, 0, 0],
    [80, 80, 0],
    [600, 0, 0],
  ],
};

/** 面板内的次级说明文字 */
function Note({ children }: { children: ReactNode }) {
  return <p className="font-mono text-[10px] leading-relaxed opacity-35">{children}</p>;
}

/** 滑块：标签 + 轨道 + 读数 */
function Slider({
  label,
  min,
  max,
  step = 1,
  value,
  disabled,
  onChange,
  display,
}: {
  label: string;
  min: number;
  max: number;
  step?: number;
  value: number;
  disabled?: boolean;
  onChange: (v: number) => void;
  display: ReactNode;
}) {
  return (
    <div className="flex items-center gap-3">
      <span className="w-8 shrink-0 font-mono text-[10px] opacity-50">{label}</span>
      <input
        type="range"
        min={min}
        max={max}
        step={step}
        value={value}
        disabled={disabled}
        onChange={(e) => onChange(Number(e.target.value))}
        className="h-1 flex-1 accent-accent disabled:opacity-40"
      />
      <span className="w-14 shrink-0 text-right font-mono text-xs tabular-nums">{display}</span>
    </div>
  );
}

export function HardwareControl({ className }: { className?: string }) {
  const [hw, setHw] = useState<HardwareState | null>(null);
  const [loading, setLoading] = useState<string | null>(null);
  const [activeVibe, setActiveVibe] = useState<string | null>(null);
  const [customMs, setCustomMs] = useState(300);
  const [customStrong, setCustomStrong] = useState(80);
  const [memResult, setMemResult] = useState<{
    freed_mb: number;
    before: { free_mb: number; available_mb: number };
    after: { free_mb: number; available_mb: number };
  } | null>(null);

  const refresh = useCallback(() => {
    fetchHardware()
      .then((d) => {
        setHw(d);
        if (!d.vibrating) setActiveVibe(null);
      })
      .catch(() => {});
  }, []);

  useEffect(() => {
    refresh();
    const t = setInterval(refresh, 2000);
    return () => clearInterval(t);
  }, [refresh]);

  const handleFlashlight = async (led: 'white' | 'yellow', on: boolean) => {
    setLoading(`flash-${led}`);
    try {
      await setFlashlight(led, on);
      refresh();
    } catch {
      /* 轮询会纠正显示状态 */
    }
    setLoading(null);
  };

  const handleStatusLed = async (on: boolean) => {
    setLoading('status-led');
    try {
      await setStatusLed(on);
      refresh();
    } catch {
      /* 轮询会纠正显示状态 */
    }
    setLoading(null);
  };

  const handleCpuStatusLedLink = async (enabled: boolean) => {
    setLoading('cpu-led-link');
    try {
      await setCpuStatusLedLink(enabled);
      refresh();
    } catch {
      /* 轮询会纠正显示状态 */
    }
    setLoading(null);
  };

  const handleBrightness = async (percent: number) => {
    setLoading('brightness');
    try {
      await setBrightness(percent);
      refresh();
    } catch {
      /* 轮询会纠正显示状态 */
    }
    setLoading(null);
  };

  const handleScreenPower = async (on: boolean) => {
    setLoading('screen');
    try {
      await setScreenPower(on);
      setTimeout(refresh, 500);
    } catch {
      /* 轮询会纠正显示状态 */
    }
    setLoading(null);
  };

  const handleChargeCurrent = async (ua: number) => {
    setLoading('charge');
    try {
      await setChargeCurrent(ua);
      refresh();
    } catch {
      /* 轮询会纠正显示状态 */
    }
    setLoading(null);
  };

  const handleChargeMode = async (powerOnly: boolean) => {
    setLoading('charge-mode');
    try {
      await setChargeMode(powerOnly);
      refresh();
    } catch {
      /* 轮询会纠正显示状态 */
    }
    setLoading(null);
  };

  const handleWifiPowerSave = async (enabled: boolean) => {
    setLoading('wifi-ps');
    try {
      await setWifiPowerSave(enabled);
      refresh();
    } catch {
      /* 轮询会纠正显示状态 */
    }
    setLoading(null);
  };

  const handleSpeakerVolume = async (percent: number) => {
    setLoading('speaker-vol');
    try {
      await setSpeakerVolume(percent);
      refresh();
    } catch {
      /* 轮询会纠正显示状态 */
    }
    setLoading(null);
  };

  const handleSpeakerMute = async (muted: boolean) => {
    setLoading('speaker-mute');
    try {
      await setSpeakerMute(muted);
      refresh();
    } catch {
      /* 轮询会纠正显示状态 */
    }
    setLoading(null);
  };

  const handleSpeakerTest = async () => {
    setLoading('speaker-test');
    try {
      await playSpeakerTest();
    } catch {
      /* 测试音失败无需提示，用户可重试 */
    }
    setLoading(null);
  };

  const handleVibeOnce = async (ms: number, _strong: number, _weak: number, label: string) => {
    setLoading(`vib-${label}`);
    try {
      await vibrate(ms);
      setActiveVibe(label);
      setTimeout(() => {
        setActiveVibe(null);
        setLoading(null);
      }, ms + 200);
    } catch {
      setLoading(null);
    }
  };

  const handleVibePattern = async (name: string, repeat: boolean) => {
    const segs = PATTERNS[name];
    if (!segs) return;
    setLoading(`vib-${name}`);
    try {
      await vibratePattern(segs, repeat);
      setActiveVibe(name);
    } catch {
      /* 轮询会纠正显示状态 */
    }
    setLoading(null);
  };

  const handleStop = async () => {
    setLoading('vib-stop');
    try {
      await vibrateStop();
      setActiveVibe(null);
    } catch {
      /* 轮询会纠正显示状态 */
    }
    setLoading(null);
  };

  const handleClearMemory = async () => {
    setLoading('clear-mem');
    setMemResult(null);
    try {
      const res = await clearMemory();
      setMemResult(res.data);
    } catch {
      /* 失败无需提示，用户可重试 */
    }
    setLoading(null);
  };

  const gridCls = `grid items-stretch gap-3 xl:gap-4 md:grid-cols-2 ${className ?? ''}`;

  if (!hw) {
    return (
      <div className={gridCls}>
        <Panel label="硬件控制" index={14}>
          <span className="font-mono text-sm opacity-30">加载硬件状态...</span>
        </Panel>
      </div>
    );
  }

  const activeChargeUa = hw.charging.target_current_max_ua || hw.charging.current_max_ua;
  const btn = 'font-mono';

  return (
    <div className={gridCls}>
      {/* 闪光灯 */}
      <Panel label="闪光灯" index={14} hint={`${hw.flashlight.max_brightness} 级`}>
        <div className="flex flex-1 flex-col justify-center gap-3">
          {(['white', 'yellow'] as const).map((led) => {
            const on = led === 'white' ? hw.flashlight.white_on : hw.flashlight.yellow_on;
            const label = led === 'white' ? '白色' : '黄色';
            const dotColor = led === 'white' ? '#ffffff' : '#fbbf24';
            return (
              <div key={led} className="flex items-center gap-3">
                <span
                  className="inline-block size-3 shrink-0 rounded-full border-2 border-default-300 transition-colors"
                  style={{
                    background: on ? dotColor : 'transparent',
                    boxShadow: on ? `0 0 10px ${dotColor}` : undefined,
                  }}
                />
                <span className="font-mono text-sm">{label}</span>
                <Chip
                  size="sm"
                  color={on ? (led === 'white' ? 'success' : 'warning') : 'default'}
                  variant="secondary"
                >
                  {on ? 'ON' : 'OFF'}
                </Chip>
                <Button
                  size="sm"
                  variant="secondary"
                  isDisabled={loading === `flash-${led}`}
                  onPress={() => handleFlashlight(led, !on)}
                  className={`${btn} ml-auto text-xs`}
                >
                  {on ? '关闭' : '开启'}
                </Button>
              </div>
            );
          })}
        </div>
        <Note>白色与黄色双色温 LED，可独立开关</Note>
      </Panel>

      {/* 状态灯 */}
      <Panel
        label="状态灯"
        index={15}
        hint={
          <Chip size="sm" color={hw.status_led.on ? 'success' : 'default'} variant="secondary">
            {hw.status_led.on ? 'ON' : 'OFF'}
          </Chip>
        }
      >
        <div className="flex items-center gap-3">
          <span
            className="inline-block size-3 shrink-0 rounded-full border-2 border-default-300 bg-white transition-opacity"
            style={{
              opacity: hw.status_led.on ? 1 : 0.2,
              boxShadow: hw.status_led.on ? '0 0 10px #ffffff' : undefined,
            }}
          />
          <span className="font-mono text-sm">white:status</span>
          <span className="ml-auto font-mono text-[10px] opacity-30">
            {hw.status_led.brightness}/{hw.status_led.max_brightness}
          </span>
        </div>
        {hw.cpu_status_led_link.enabled && (
          <div className="dm-inset flex items-center justify-between gap-2 px-2.5 py-1.5 font-mono text-[10px]">
            <span className="opacity-50">CPU 联动亮度</span>
            <span className="text-warning">
              {hw.cpu_status_led_link.link_brightness_pct}% · CPU{' '}
              {hw.cpu_status_led_link.smoothed_cpu_pct.toFixed(0)}%
            </span>
          </div>
        )}
        <div className="mt-auto flex flex-wrap gap-2">
          <Button
            size="sm"
            variant="secondary"
            isDisabled={loading === 'cpu-led-link'}
            onPress={() => handleCpuStatusLedLink(!hw.cpu_status_led_link.enabled)}
            className={`${btn} flex-1 text-xs`}
          >
            {hw.cpu_status_led_link.enabled ? '关闭 CPU 联动' : 'CPU 使用率联动'}
          </Button>
          <Button
            size="sm"
            variant="secondary"
            isDisabled={loading === 'status-led'}
            onPress={() => handleStatusLed(!hw.status_led.on)}
            className={`${btn} flex-1 text-xs`}
          >
            {hw.status_led.on ? '关闭状态灯' : '开启状态灯'}
          </Button>
        </div>
        <Note>联动：{hw.cpu_status_led_link.threshold_pct}% 以下不亮，超过后平滑增亮</Note>
      </Panel>

      {/* 充电电流 */}
      <Panel
        label="充电电流"
        index={16}
        hint={
          <>
            {hw.charging.charge_mode === 'power_only' && (
              <Chip size="sm" color="warning" variant="secondary">
                仅供电
              </Chip>
            )}
            <Chip
              size="sm"
              color={hw.charging.charger_online ? 'success' : 'default'}
              variant="secondary"
            >
              {chargeSourceLabel(hw.charging.charge_source)}
            </Chip>
          </>
        }
      >
        <div className="flex gap-2">
          <Button
            size="sm"
            variant={hw.charging.charge_mode === 'normal' ? 'secondary' : 'ghost'}
            isDisabled={loading === 'charge-mode'}
            onPress={() => handleChargeMode(false)}
            className={`${btn} flex-1 text-xs`}
          >
            正常充电
          </Button>
          <Button
            size="sm"
            variant={hw.charging.charge_mode === 'power_only' ? 'secondary' : 'ghost'}
            isDisabled={loading === 'charge-mode'}
            onPress={() => handleChargeMode(true)}
            className={`${btn} flex-1 text-xs`}
          >
            仅供电
          </Button>
        </div>

        <div className="flex flex-wrap items-baseline gap-x-2 font-mono">
          <span className="dm-hero text-xl font-light">{fmtChargeUa(activeChargeUa)}</span>
          <span className="text-[10px] opacity-40">{chargeModeLabel(hw.charging.charge_mode)}</span>
          {hw.charging.target_current_max_ua > 0 &&
            hw.charging.current_max_ua !== hw.charging.target_current_max_ua && (
              <span className="text-[10px] opacity-40">
                实际 {fmtChargeUa(hw.charging.current_max_ua)}
              </span>
            )}
          {hw.charging.charger_online &&
            hw.charging.power_w > 0 &&
            hw.charging.charge_mode === 'normal' && (
              <span className="text-[10px] opacity-40">
                实时 {hw.charging.power_w.toFixed(1)}W ·{' '}
                {Math.round(hw.charging.current_now_ua / 1000)}mA
              </span>
            )}
        </div>

        <div className="flex flex-wrap gap-2">
          {CHARGE_PRESETS.map((p) => {
            const wirelessLimited = hw.charging.charge_source === 'wireless' && p.wiredOnly;
            const powerOnly = hw.charging.charge_mode === 'power_only';
            return (
              <Button
                key={p.ua}
                size="sm"
                variant={isChargePresetSelected(activeChargeUa, p.ua) ? 'secondary' : 'ghost'}
                isDisabled={loading === 'charge' || wirelessLimited || powerOnly}
                onPress={() => handleChargeCurrent(p.ua)}
                className={`${btn} text-xs`}
              >
                {p.label}
              </Button>
            );
          })}
        </div>
        <Note>
          有线最大 18W · 无线最大 10W · 仅供电时挂起电池充电
          {hw.charging.charger_online && hw.charging.usb_type ? ` · ${hw.charging.usb_type}` : ''}
        </Note>
      </Panel>

      {/* WiFi 省电 */}
      <Panel
        label="WiFi 省电"
        index={17}
        hint={
          <Chip
            size="sm"
            color={hw.wifi_power_save.enabled ? 'warning' : 'success'}
            variant="secondary"
          >
            {hw.wifi_power_save.enabled ? '开启' : '关闭'}
          </Chip>
        }
      >
        <div className="dm-inset flex items-center justify-between gap-2 px-2.5 py-2 font-mono text-[11px]">
          <span className="opacity-50">接口</span>
          <span>{hw.wifi_power_save.iface}</span>
        </div>
        <Button
          size="md"
          variant="secondary"
          isDisabled={loading === 'wifi-ps'}
          onPress={() => handleWifiPowerSave(!hw.wifi_power_save.enabled)}
          className={`${btn} mt-auto text-sm`}
        >
          {hw.wifi_power_save.enabled ? '关闭省电模式' : '开启省电模式'}
        </Button>
        <Note>省电模式会降低无线唤醒频率，可能增加延迟</Note>
      </Panel>

      {/* 屏幕 */}
      <Panel
        label="屏幕"
        index={18}
        hint={
          <Chip size="sm" color={hw.screen_on ? 'success' : 'default'} variant="secondary">
            {hw.screen_on ? '亮屏' : '息屏'}
          </Chip>
        }
      >
        <div className="flex items-baseline gap-2">
          <span className="dm-hero text-xl font-light">{hw.brightness.percent}</span>
          <span className="font-mono text-[10px] opacity-40">% 亮度</span>
          <span className="ml-auto font-mono text-[10px] opacity-30">
            {hw.brightness.current}/{hw.brightness.max}
          </span>
        </div>
        <Slider
          label="亮度"
          min={0}
          max={100}
          value={hw.brightness.percent}
          disabled={loading === 'brightness' || !hw.screen_on}
          onChange={handleBrightness}
          display={`${hw.brightness.percent}%`}
        />
        <div className="flex gap-2">
          {BRIGHTNESS_PRESETS.map((pct) => (
            <Button
              key={pct}
              size="sm"
              variant={hw.brightness.percent === pct ? 'secondary' : 'ghost'}
              isDisabled={loading === 'brightness' || !hw.screen_on}
              onPress={() => handleBrightness(pct)}
              className={`${btn} flex-1 text-xs`}
            >
              {pct === 0 ? '关' : `${pct}%`}
            </Button>
          ))}
        </div>
        <Button
          size="sm"
          variant="secondary"
          isDisabled={loading === 'screen'}
          onPress={() => handleScreenPower(!hw.screen_on)}
          className={`${btn} mt-auto text-xs`}
        >
          {hw.screen_on ? '息屏' : '亮屏'}
        </Button>
      </Panel>

      {/* 扬声器 */}
      <Panel
        label="扬声器"
        index={19}
        hint={
          <>
            {hw.speaker.muted && (
              <Chip size="sm" color="warning" variant="secondary">
                静音
              </Chip>
            )}
            <Chip
              size="sm"
              color={hw.speaker.available ? 'success' : 'default'}
              variant="secondary"
            >
              {hw.speaker.available ? hw.speaker.backend : '不可用'}
            </Chip>
          </>
        }
      >
        <div className="flex items-baseline gap-2">
          <span className="dm-hero text-xl font-light">
            {hw.speaker.muted ? 0 : hw.speaker.volume_percent}
          </span>
          <span className="font-mono text-[10px] opacity-40">% 音量</span>
          <span className="ml-auto truncate font-mono text-[10px] opacity-30">
            {hw.speaker.sink_name}
          </span>
        </div>
        <Slider
          label="音量"
          min={0}
          max={100}
          value={hw.speaker.muted ? 0 : hw.speaker.volume_percent}
          disabled={loading?.startsWith('speaker') || !hw.speaker.available}
          onChange={handleSpeakerVolume}
          display={`${hw.speaker.muted ? 0 : hw.speaker.volume_percent}%`}
        />
        <div className="flex gap-2">
          {SPEAKER_PRESETS.map((pct) => (
            <Button
              key={pct}
              size="sm"
              variant={
                !hw.speaker.muted && hw.speaker.volume_percent === pct ? 'secondary' : 'ghost'
              }
              isDisabled={loading?.startsWith('speaker') || !hw.speaker.available}
              onPress={() => handleSpeakerVolume(pct)}
              className={`${btn} flex-1 text-xs`}
            >
              {pct === 0 ? '关' : `${pct}%`}
            </Button>
          ))}
        </div>
        <div className="mt-auto flex gap-2">
          <Button
            size="sm"
            variant={hw.speaker.muted ? 'secondary' : 'ghost'}
            isDisabled={loading?.startsWith('speaker') || !hw.speaker.available}
            onPress={() => handleSpeakerMute(!hw.speaker.muted)}
            className={`${btn} flex-1 text-xs`}
          >
            {hw.speaker.muted ? '取消静音' : '静音'}
          </Button>
          <Button
            size="sm"
            variant="secondary"
            isDisabled={loading === 'speaker-test' || !hw.speaker.available}
            onPress={handleSpeakerTest}
            className={`${btn} flex-1 text-xs`}
          >
            {loading === 'speaker-test' ? '播放中...' : '测试音'}
          </Button>
        </div>
      </Panel>

      {/* 振动马达 */}
      <Panel
        label="振动马达"
        index={20}
        hint={
          activeVibe && hw.vibrating ? (
            <Button
              size="sm"
              variant="danger"
              isDisabled={loading === 'vib-stop'}
              onPress={handleStop}
              className="h-6 min-w-0 px-2 font-mono text-xs"
            >
              ■ 停止
            </Button>
          ) : activeVibe ? (
            <span className="text-accent">{activeVibe} 完成</span>
          ) : undefined
        }
      >
        <div className="dm-inset flex flex-col gap-2.5 p-3">
          <Slider
            label="时长"
            min={50}
            max={3000}
            step={50}
            value={customMs}
            onChange={setCustomMs}
            display={`${customMs}ms`}
          />
          <Slider
            label="强度"
            min={10}
            max={100}
            step={5}
            value={customStrong}
            onChange={setCustomStrong}
            display={`${customStrong}%`}
          />
          <Button
            size="sm"
            variant="secondary"
            isDisabled={loading?.startsWith('vib-')}
            onPress={() => handleVibeOnce(customMs, customStrong, 0, 'custom')}
            className={`${btn} text-xs`}
          >
            振动 {customMs}ms
          </Button>
        </div>

        <div className="grid grid-cols-4 gap-2 sm:grid-cols-7">
          {VIBE_PRESETS.map((v) => {
            const isPattern = v.ms === 0;
            return (
              <Button
                key={v.name}
                size="sm"
                variant={activeVibe === v.name ? 'secondary' : 'ghost'}
                isDisabled={loading === `vib-${v.name}`}
                onPress={() =>
                  isPattern
                    ? handleVibePattern(v.name, v.name !== '双击')
                    : handleVibeOnce(v.ms, v.strong, v.weak, v.name)
                }
                className={`${btn} min-w-0 px-1 text-xs`}
              >
                {v.name}
              </Button>
            );
          })}
        </div>
      </Panel>

      {/* 系统工具 */}
      <Panel label="系统工具" index={21}>
        <div className="flex flex-1 flex-col items-center justify-center gap-3 py-2">
          <Button
            size="md"
            variant="secondary"
            isDisabled={loading === 'clear-mem'}
            onPress={handleClearMemory}
            className={`${btn} text-sm`}
          >
            {loading === 'clear-mem' ? '清理中...' : '一键清理内存'}
          </Button>
          {memResult ? (
            <div className="flex flex-col items-center gap-1.5">
              <div className="flex items-center gap-2 font-mono text-xs">
                <Chip size="sm" color="success" variant="secondary">
                  释放 {fmtMem(memResult.freed_mb)}
                </Chip>
                <span className="opacity-50">
                  {fmtMem(memResult.before.available_mb)} → {fmtMem(memResult.after.available_mb)}
                </span>
              </div>
              <span className="font-mono text-[10px] opacity-35">以上为「可用内存」变化</span>
            </div>
          ) : (
            <span className="font-mono text-[10px] opacity-30">释放页缓存与可回收内存</span>
          )}
        </div>
      </Panel>
    </div>
  );
}

import { useState, useEffect, useCallback } from 'react';
import { Card, Button, Chip } from '@heroui/react';
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
} from '../api';
import { fmtChargeUa, chargeSourceLabel, chargeModeLabel, isChargePresetSelected } from './utils';

interface HardwareState {
  flashlight: { white_on: boolean; yellow_on: boolean; max_brightness: number };
  status_led: { on: boolean; brightness: number; max_brightness: number; percent: number };
  cpu_status_led_link: { enabled: boolean };
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
}

const BRIGHTNESS_PRESETS = [0, 25, 50, 75, 100];

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
  '双击': [[100, 80, 0], [80, 0, 0], [100, 80, 0]],
  '心跳': [[100, 90, 0], [100, 0, 0], [60, 70, 0], [500, 0, 0]],
  'SOS': [[80, 80, 0], [80, 0, 0], [80, 80, 0], [80, 0, 0], [80, 80, 0], [200, 0, 0], [200, 80, 0], [200, 0, 0], [200, 80, 0], [200, 0, 0], [200, 80, 0], [200, 0, 0], [80, 80, 0], [80, 0, 0], [80, 80, 0], [80, 0, 0], [80, 80, 0], [600, 0, 0]],
};

function formatUa(ua: number) {
  return fmtChargeUa(ua);
}

export function HardwareControl({ embedded = false }: { embedded?: boolean }) {
  const [hw, setHw] = useState<HardwareState | null>(null);
  const [loading, setLoading] = useState<string | null>(null);
  const [activeVibe, setActiveVibe] = useState<string | null>(null);
  const [customMs, setCustomMs] = useState(300);
  const [customStrong, setCustomStrong] = useState(80);
  const [memResult, setMemResult] = useState<{ freed_mb: number; before: { free_mb: number; available_mb: number }; after: { free_mb: number; available_mb: number } } | null>(null);

  const refresh = useCallback(() => {
    fetchHardware().then(d => {
      setHw(d);
      if (!d.vibrating) setActiveVibe(null);
    }).catch(() => {});
  }, []);

  useEffect(() => {
    refresh();
    const t = setInterval(refresh, 2000);
    return () => clearInterval(t);
  }, [refresh]);

  const handleFlashlight = async (led: 'white' | 'yellow', on: boolean) => {
    setLoading(`flash-${led}`);
    try { await setFlashlight(led, on); refresh(); } catch {}
    setLoading(null);
  };

  const handleStatusLed = async (on: boolean) => {
    setLoading('status-led');
    try { await setStatusLed(on); refresh(); } catch {}
    setLoading(null);
  };

  const handleCpuStatusLedLink = async (enabled: boolean) => {
    setLoading('cpu-led-link');
    try { await setCpuStatusLedLink(enabled); refresh(); } catch {}
    setLoading(null);
  };

  const handleBrightness = async (percent: number) => {
    setLoading('brightness');
    try { await setBrightness(percent); refresh(); } catch {}
    setLoading(null);
  };

  const handleScreenPower = async (on: boolean) => {
    setLoading('screen');
    try { await setScreenPower(on); setTimeout(refresh, 500); } catch {}
    setLoading(null);
  };

  const handleChargeCurrent = async (ua: number) => {
    setLoading('charge');
    try { await setChargeCurrent(ua); refresh(); } catch {}
    setLoading(null);
  };

  const handleChargeMode = async (powerOnly: boolean) => {
    setLoading('charge-mode');
    try { await setChargeMode(powerOnly); refresh(); } catch {}
    setLoading(null);
  };

  const handleWifiPowerSave = async (enabled: boolean) => {
    setLoading('wifi-ps');
    try { await setWifiPowerSave(enabled); refresh(); } catch {}
    setLoading(null);
  };

  const handleVibeOnce = async (ms: number, _strong: number, _weak: number, label: string) => {
    setLoading(`vib-${label}`);
    try {
      await vibrate(ms);
      setActiveVibe(label);
      setTimeout(() => { setActiveVibe(null); setLoading(null); }, ms + 200);
    } catch { setLoading(null); }
  };

  const handleVibePattern = async (name: string, repeat: boolean) => {
    const segs = PATTERNS[name];
    if (!segs) return;
    setLoading(`vib-${name}`);
    try {
      await vibratePattern(segs, repeat);
      setActiveVibe(name);
    } catch {}
    setLoading(null);
  };

  const handleStop = async () => {
    setLoading('vib-stop');
    try { await vibrateStop(); setActiveVibe(null); } catch {}
    setLoading(null);
  };

  const handleClearMemory = async () => {
    setLoading('clear-mem');
    setMemResult(null);
    try {
      const res = await clearMemory();
      setMemResult(res.data);
    } catch {}
    setLoading(null);
  };

  if (!hw) {
    const loading = (
      <Card className={`p-6 flex items-center justify-center ${embedded ? 'md:col-span-2 xl:col-span-3' : ''}`}>
        <span className="font-mono text-sm opacity-30">加载硬件状态...</span>
      </Card>
    );
    return embedded ? loading : <div className="grid grid-cols-1 sm:grid-cols-2 xl:grid-cols-3 gap-3">{loading}</div>;
  }

  const span2 = embedded ? 'md:col-span-2 xl:col-span-2' : '';
  const spanFull = embedded ? 'md:col-span-2 xl:col-span-3' : 'sm:col-span-2 xl:col-span-3';
  const activeChargeUa = hw.charging.target_current_max_ua || hw.charging.current_max_ua;

  const cards = (
    <>
      {/* 闪光灯 */}
      <Card className="p-4 sm:p-5 flex flex-col gap-4">
        <span className="text-[10px] font-mono uppercase tracking-widest opacity-50">闪光灯</span>
        {(['white', 'yellow'] as const).map(led => {
          const on = led === 'white' ? hw.flashlight.white_on : hw.flashlight.yellow_on;
          const label = led === 'white' ? '白色' : '黄色';
          const dotColor = led === 'white' ? '#fff' : '#fbbf24';
          return (
            <div key={led} className="flex items-center justify-between">
              <div className="flex items-center gap-2">
                <span className="inline-block w-3 h-3 rounded-full border-2 border-default-300" style={{ background: on ? dotColor : 'transparent' }} />
                <span className="font-mono text-sm">{label}</span>
                <Chip size="sm" color={on ? (led === 'white' ? 'success' : 'warning') : 'default'} variant="secondary">{on ? 'ON' : 'OFF'}</Chip>
              </div>
              <Button size="sm" variant={on ? 'danger' : 'secondary'} isDisabled={loading === `flash-${led}`} onPress={() => handleFlashlight(led, !on)} className="font-mono text-xs">{on ? '关闭' : '开启'}</Button>
            </div>
          );
        })}
      </Card>

      {/* 状态灯 */}
      <Card className="p-4 sm:p-5 flex flex-col gap-4">
        <div className="flex items-center justify-between">
          <span className="text-[10px] font-mono uppercase tracking-widest opacity-50">状态灯</span>
          <div className="flex items-center gap-2">
            {hw.cpu_status_led_link.enabled && (
              <Chip size="sm" color="warning" variant="secondary">CPU联动</Chip>
            )}
            <Chip size="sm" color={hw.status_led.on ? 'success' : 'default'} variant="secondary">{hw.status_led.on ? 'ON' : 'OFF'}</Chip>
          </div>
        </div>
        <div className="flex items-center gap-3">
          <span className="inline-block w-3 h-3 rounded-full border-2 border-default-300 bg-white" style={{ opacity: hw.status_led.on ? 1 : 0.2 }} />
          <span className="font-mono text-sm">white:status</span>
          <span className="font-mono text-[10px] opacity-30">{hw.status_led.brightness}/{hw.status_led.max_brightness}</span>
        </div>
        <Button
          size="sm"
          variant={hw.cpu_status_led_link.enabled ? 'danger' : 'secondary'}
          isDisabled={loading === 'cpu-led-link'}
          onPress={() => handleCpuStatusLedLink(!hw.cpu_status_led_link.enabled)}
          className="font-mono text-xs"
        >
          {hw.cpu_status_led_link.enabled ? '关闭 CPU 联动' : 'CPU 使用率联动'}
        </Button>
        <Button size="md" variant={hw.status_led.on ? 'danger' : 'secondary'} isDisabled={loading === 'status-led'} onPress={() => handleStatusLed(!hw.status_led.on)} className="font-mono text-sm">
          {hw.status_led.on ? '关闭状态灯' : '开启状态灯'}
        </Button>
      </Card>

      {/* 充电 — 短卡片优先，避免高卡片留洞 */}
      <Card className="p-4 sm:p-5 flex flex-col gap-4">
        <div className="flex items-center justify-between">
          <span className="text-[10px] font-mono uppercase tracking-widest opacity-50">充电电流</span>
          <div className="flex items-center gap-2">
            {hw.charging.charge_mode === 'power_only' && (
              <Chip size="sm" color="warning" variant="secondary">仅供电</Chip>
            )}
            <Chip size="sm" color={hw.charging.charger_online ? 'success' : 'default'} variant="secondary">
              {chargeSourceLabel(hw.charging.charge_source)}
            </Chip>
          </div>
        </div>
        <div className="flex gap-2">
          <Button
            size="sm"
            variant={hw.charging.charge_mode === 'normal' ? 'secondary' : 'ghost'}
            isDisabled={loading === 'charge-mode'}
            onPress={() => handleChargeMode(false)}
            className="flex-1 font-mono text-xs"
          >
            正常充电
          </Button>
          <Button
            size="sm"
            variant={hw.charging.charge_mode === 'power_only' ? 'secondary' : 'ghost'}
            isDisabled={loading === 'charge-mode'}
            onPress={() => handleChargeMode(true)}
            className="flex-1 font-mono text-xs"
          >
            仅供电
          </Button>
        </div>
        <div className="font-mono text-sm">
          目标 <span className="text-lg">{formatUa(activeChargeUa)}</span>
          <span className="ml-2 text-[10px] opacity-40">{chargeModeLabel(hw.charging.charge_mode)}</span>
          {hw.charging.target_current_max_ua > 0 && hw.charging.current_max_ua !== hw.charging.target_current_max_ua && (
            <span className="ml-2 text-[10px] opacity-40">
              实际 {formatUa(hw.charging.current_max_ua)}
            </span>
          )}
          {hw.charging.charger_online && hw.charging.power_w > 0 && hw.charging.charge_mode === 'normal' && (
            <span className="ml-2 text-[10px] opacity-40">
              实时 {hw.charging.power_w.toFixed(1)}W · {Math.round(hw.charging.current_now_ua / 1000)}mA
            </span>
          )}
        </div>
        <span className="font-mono text-[10px] opacity-30">
          有线最大 18W · 无线最大 10W · 仅供电时挂起电池充电
          {hw.charging.charger_online && hw.charging.usb_type && (
            <> · {hw.charging.usb_type}</>
          )}
        </span>
        <div className="flex flex-wrap gap-2">
          {CHARGE_PRESETS.map(p => {
            const wirelessLimited = hw.charging.charge_source === 'wireless' && p.wiredOnly;
            const powerOnly = hw.charging.charge_mode === 'power_only';
            return (
              <Button
                key={p.ua}
                size="sm"
                variant={isChargePresetSelected(activeChargeUa, p.ua) ? 'secondary' : 'ghost'}
                isDisabled={loading === 'charge' || wirelessLimited || powerOnly}
                onPress={() => handleChargeCurrent(p.ua)}
                className="font-mono text-xs"
              >
                {p.label}
              </Button>
            );
          })}
        </div>
      </Card>

      {/* WiFi 省电 */}
      <Card className="p-4 sm:p-5 flex flex-col gap-4">
        <div className="flex items-center justify-between">
          <span className="text-[10px] font-mono uppercase tracking-widest opacity-50">WiFi 省电</span>
          <Chip size="sm" color={hw.wifi_power_save.enabled ? 'warning' : 'success'} variant="secondary">
            {hw.wifi_power_save.enabled ? '开启' : '关闭'}
          </Chip>
        </div>
        <span className="font-mono text-[10px] opacity-30">{hw.wifi_power_save.iface}</span>
        <Button
          size="md"
          variant={hw.wifi_power_save.enabled ? 'danger' : 'secondary'}
          isDisabled={loading === 'wifi-ps'}
          onPress={() => handleWifiPowerSave(!hw.wifi_power_save.enabled)}
          className="font-mono text-sm"
        >
          {hw.wifi_power_save.enabled ? '关闭省电模式' : '开启省电模式'}
        </Button>
      </Card>

      {/* 屏幕 — 较高，占 2 列与 WiFi 同行 */}
      <Card className={`p-4 sm:p-5 flex flex-col gap-4 ${span2}`}>
        <div className="flex items-center justify-between">
          <span className="text-[10px] font-mono uppercase tracking-widest opacity-50">屏幕</span>
          <Chip size="sm" color={hw.screen_on ? 'success' : 'default'} variant="secondary">{hw.screen_on ? '亮屏' : '息屏'}</Chip>
        </div>
        <Button size="md" variant={hw.screen_on ? 'danger' : 'secondary'} isDisabled={loading === 'screen'} onPress={() => handleScreenPower(!hw.screen_on)} className="font-mono text-sm">{hw.screen_on ? '息屏' : '亮屏'}</Button>
        <div className="flex items-center gap-3">
          <span className="font-mono text-xl font-light">{hw.brightness.percent}<span className="text-[10px] opacity-50">%</span></span>
          <span className="font-mono text-[10px] opacity-30">{hw.brightness.current}/{hw.brightness.max}</span>
        </div>
        <div className="flex gap-2">
          {BRIGHTNESS_PRESETS.map(pct => (
            <Button key={pct} size="sm" variant={hw.brightness.percent === pct ? 'secondary' : 'ghost'} isDisabled={loading === 'brightness' || !hw.screen_on} onPress={() => handleBrightness(pct)} className="flex-1 font-mono text-xs">{pct === 0 ? '关' : `${pct}%`}</Button>
          ))}
        </div>
        <input type="range" min={0} max={100} value={hw.brightness.percent} onChange={e => handleBrightness(Number(e.target.value))} disabled={loading === 'brightness' || !hw.screen_on} className="w-full accent-accent h-1.5" />
      </Card>

      {/* 振动马达 */}
      <Card className={`p-4 sm:p-5 flex flex-col gap-3 ${spanFull}`}>
        <div className="flex items-center justify-between">
          <span className="text-[10px] font-mono uppercase tracking-widest opacity-50">振动马达</span>
          {activeVibe && hw.vibrating && (
            <Button size="sm" variant="danger" isDisabled={loading === 'vib-stop'} onPress={handleStop} className="font-mono text-xs h-6 min-w-0 px-2">■ 停止</Button>
          )}
        </div>

        <div className="flex flex-col gap-2 p-3 rounded-lg bg-default-50">
          <div className="flex items-center gap-3">
            <span className="font-mono text-[10px] opacity-50 w-10">时长</span>
            <input type="range" min={50} max={3000} step={50} value={customMs} onChange={e => setCustomMs(Number(e.target.value))} className="flex-1 accent-accent h-1.5" />
            <span className="font-mono text-xs w-14 text-right">{customMs}ms</span>
          </div>
          <div className="flex items-center gap-3">
            <span className="font-mono text-[10px] opacity-50 w-10">强度</span>
            <input type="range" min={10} max={100} step={5} value={customStrong} onChange={e => setCustomStrong(Number(e.target.value))} className="flex-1 accent-accent h-1.5" />
            <span className="font-mono text-xs w-14 text-right">{customStrong}%</span>
          </div>
          <Button
            size="md"
            variant="secondary"
            isDisabled={loading?.startsWith('vib-')}
            onPress={() => handleVibeOnce(customMs, customStrong, 0, 'custom')}
            className="font-mono text-sm mt-1"
          >
            振动 {customMs}ms
          </Button>
        </div>

        <div className="grid grid-cols-4 sm:grid-cols-7 gap-2">
          {VIBE_PRESETS.map(v => {
            const isPattern = v.ms === 0;
            const label = v.name;
            return (
              <Button
                key={v.name}
                size="sm"
                variant={activeVibe === v.name ? 'secondary' : 'ghost'}
                isDisabled={loading === `vib-${v.name}`}
                onPress={() => isPattern ? handleVibePattern(v.name, v.name !== '双击') : handleVibeOnce(v.ms, v.strong, v.weak, v.name)}
                className="font-mono text-xs"
              >
                {label}
              </Button>
            );
          })}
        </div>

        {activeVibe && (
          <div className="flex items-center gap-2 font-mono text-[10px] text-accent">
            <span className="inline-block w-1.5 h-1.5 rounded-full bg-accent animate-pulse" />
            {hw.vibrating ? `正在振动: ${activeVibe}` : `${activeVibe} 完成`}
          </div>
        )}
      </Card>

      {/* 系统工具 */}
      <Card className={`p-4 sm:p-5 flex flex-col gap-4 ${spanFull}`}>
        <span className="text-[10px] font-mono uppercase tracking-widest opacity-50">系统工具</span>
        <div className="flex items-center gap-3">
          <Button
            size="md"
            variant="secondary"
            isDisabled={loading === 'clear-mem'}
            onPress={handleClearMemory}
            className="font-mono text-sm"
          >
            {loading === 'clear-mem' ? '清理中...' : '🧹 一键清理内存'}
          </Button>
          {memResult && (
            <div className="flex items-center gap-2 font-mono text-xs">
              <Chip size="sm" color="success" variant="secondary">释放 {memResult.freed_mb} MB</Chip>
              <span className="opacity-50">{memResult.before.available_mb}MB → {memResult.after.available_mb}MB</span>
            </div>
          )}
        </div>
      </Card>
    </>
  );

  if (embedded) return cards;
  return <div className="grid grid-cols-1 sm:grid-cols-2 xl:grid-cols-3 gap-3">{cards}</div>;
}

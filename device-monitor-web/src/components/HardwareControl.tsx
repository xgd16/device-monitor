import { useState, useEffect, useCallback } from 'react';
import { Card, Button, Chip } from '@heroui/react';
import { fetchHardware, setFlashlight, setBrightness, setScreenPower, vibrate, vibratePattern, vibrateStop, clearMemory } from '../api';

interface HardwareState {
  flashlight: { white_on: boolean; yellow_on: boolean; max_brightness: number };
  brightness: { current: number; max: number; percent: number };
  screen_on: boolean;
  vibrating: boolean;
}

const BRIGHTNESS_PRESETS = [0, 25, 50, 75, 100];

const VIBE_PRESETS: { name: string; ms: number; strong: number; weak: number }[] = [
  { name: '轻触', ms: 50, strong: 40, weak: 0 },
  { name: '短震', ms: 150, strong: 80, weak: 0 },
  { name: '中震', ms: 400, strong: 80, weak: 0 },
  { name: '长震', ms: 800, strong: 80, weak: 0 },
  { name: '双击', ms: 0, strong: 80, weak: 0 },  // special: pattern
  { name: '心跳', ms: 0, strong: 90, weak: 0 },  // special: pattern
  { name: 'SOS',  ms: 0, strong: 80, weak: 0 },  // special: pattern
];

// 模式段: [duration, strong%, weak%]
const PATTERNS: Record<string, [number, number, number][]> = {
  '双击': [[100,80,0],[80,0,0],[100,80,0]],
  '心跳': [[100,90,0],[100,0,0],[60,70,0],[500,0,0]],
  'SOS': [[80,80,0],[80,0,0],[80,80,0],[80,0,0],[80,80,0],[200,0,0],[200,80,0],[200,0,0],[200,80,0],[200,0,0],[200,80,0],[200,0,0],[80,80,0],[80,0,0],[80,80,0],[80,0,0],[80,80,0],[600,0,0]],
};

export function HardwareControl() {
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

  // 单次振动
  const handleVibeOnce = async (ms: number, _strong: number, _weak: number, label: string) => {
    setLoading(`vib-${label}`);
    try {
      await vibrate(ms);
      setActiveVibe(label);
      setTimeout(() => { setActiveVibe(null); setLoading(null); }, ms + 200);
    } catch { setLoading(null); }
  };

  // 模式振动
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

  // 停止
  const handleStop = async () => {
    setLoading('vib-stop');
    try { await vibrateStop(); setActiveVibe(null); } catch {}
    setLoading(null);
  };

  // 清理内存
  const handleClearMemory = async () => {
    setLoading('clear-mem');
    setMemResult(null);
    try {
      const res = await clearMemory();
      setMemResult(res.data);
    } catch {}
    setLoading(null);
  };

  if (!hw) return <div className="p-6 text-center font-mono text-sm opacity-30">加载硬件状态...</div>;

  return (
    <div className="grid grid-cols-1 sm:grid-cols-2 gap-3">
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

      {/* 屏幕 */}
      <Card className="p-4 sm:p-5 flex flex-col gap-4">
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
      <Card className="p-4 sm:p-5 flex flex-col gap-3 sm:col-span-2">
        <div className="flex items-center justify-between">
          <span className="text-[10px] font-mono uppercase tracking-widest opacity-50">振动马达</span>
          {activeVibe && hw.vibrating && (
            <Button size="sm" variant="danger" isDisabled={loading === 'vib-stop'} onPress={handleStop} className="font-mono text-xs h-6 min-w-0 px-2">■ 停止</Button>
          )}
        </div>

        {/* 自定义振动 */}
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

        {/* 预设 */}
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
      <Card className="p-4 sm:p-5 flex flex-col gap-4 sm:col-span-2">
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
    </div>
  );
}

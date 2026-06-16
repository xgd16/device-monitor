import { useState, useEffect, useCallback } from 'react';
import { Card, Button, Chip, ProgressBar } from '@heroui/react';
import { fetchHardware, setGpuMaxFreq } from '../api';
import { TrendChart } from './TrendChart';

interface GpuState {
  cur_freq_mhz: number;
  min_freq_mhz: number;
  max_freq_mhz: number;
  governor: string;
  available_freqs_mhz: number[];
}

export function GpuMonitorCard() {
  const [gpu, setGpu] = useState<GpuState | null>(null);
  const [loading, setLoading] = useState(false);
  const [history, setHistory] = useState<number[]>([]);
  const [timestamps, setTimestamps] = useState<number[]>([]);

  const refresh = useCallback(() => {
    fetchHardware()
      .then(d => {
        setGpu(d.gpu);
        const now = Math.floor(Date.now() / 1000);
        setHistory(prev => [...prev.slice(-119), d.gpu.cur_freq_mhz]);
        setTimestamps(prev => [...prev.slice(-119), now]);
      })
      .catch(() => {});
  }, []);

  useEffect(() => {
    refresh();
    const t = setInterval(refresh, 2000);
    return () => clearInterval(t);
  }, [refresh]);

  const handleMaxFreq = async (mhz: number) => {
    setLoading(true);
    try {
      await setGpuMaxFreq(mhz);
      refresh();
    } catch {}
    setLoading(false);
  };

  if (!gpu) {
    return (
      <Card className="p-4 sm:p-5 flex items-center justify-center min-h-[180px]">
        <span className="font-mono text-sm opacity-30">加载 GPU 数据...</span>
      </Card>
    );
  }

  const freqs = gpu.available_freqs_mhz.length > 0
    ? gpu.available_freqs_mhz
    : [257, 342, 414, 520, 596, 675, 710];
  const hwPeak = Math.max(...freqs);
  const isIdleMin = gpu.cur_freq_mhz <= gpu.min_freq_mhz;
  const isCapped = gpu.max_freq_mhz < hwPeak;
  const usagePct = gpu.max_freq_mhz > 0
    ? Math.min((gpu.cur_freq_mhz / gpu.max_freq_mhz) * 100, 100)
    : 0;

  return (
    <Card className="p-4 sm:p-5 flex flex-col gap-4">
      <div className="flex items-center justify-between">
        <span className="text-[10px] font-mono uppercase tracking-widest opacity-50">GPU 频率</span>
        <div className="flex items-center gap-1.5">
          {isCapped && (
            <Chip size="sm" color="warning" variant="secondary" className="font-mono text-[10px]">已限频</Chip>
          )}
          {isIdleMin && !isCapped && (
            <Chip size="sm" variant="secondary" className="font-mono text-[10px]">空闲</Chip>
          )}
          <Chip size="sm" variant="secondary" className="font-mono text-[10px]">{gpu.governor}</Chip>
        </div>
      </div>

      <div className="flex items-end justify-between gap-3">
        <div>
          <div className="flex items-baseline gap-2">
            <span className="font-mono text-3xl font-light">{gpu.cur_freq_mhz}</span>
            <span className="font-mono text-[10px] opacity-50">MHz 当前</span>
          </div>
          <span className="font-mono text-[10px] opacity-30">
            上限 {gpu.max_freq_mhz} MHz · 最低 {gpu.min_freq_mhz} MHz · 峰值 {hwPeak} MHz
          </span>
          {isIdleMin && (
            <span className="block font-mono text-[10px] opacity-40 mt-1">GPU 空闲时维持在最低档，负载升高会自动升频</span>
          )}
        </div>
        <div className="text-right font-mono text-[10px] opacity-40">
          <div>相对上限 {usagePct.toFixed(0)}%</div>
        </div>
      </div>

      <ProgressBar value={usagePct} size="sm" color="accent">
        <ProgressBar.Track>
          <ProgressBar.Fill />
        </ProgressBar.Track>
      </ProgressBar>

      <TrendChart data={history} timestamps={timestamps} variant="gpu" unit=" MHz" height={88} />

      <div className="flex flex-wrap gap-1.5">
        {freqs.map(mhz => (
          <Button
            key={mhz}
            size="sm"
            variant={gpu.max_freq_mhz === mhz ? 'secondary' : 'ghost'}
            isDisabled={loading}
            onPress={() => handleMaxFreq(mhz)}
            className="font-mono text-[10px] min-w-0 px-2 h-7"
          >
            {mhz}
          </Button>
        ))}
      </div>
    </Card>
  );
}

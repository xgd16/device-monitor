import { useState, useEffect, useCallback } from 'react';
import { Button, Chip } from '@heroui/react';
import { MeterRow, Panel, StatCell } from './Panel';
import { TrendChart } from './TrendChart';
import { fetchHardware, setGpuMaxFreq } from '../api';

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
      .then((d) => {
        setGpu(d.gpu);
        const now = Math.floor(Date.now() / 1000);
        setHistory((prev) => [...prev.slice(-119), d.gpu.cur_freq_mhz]);
        setTimestamps((prev) => [...prev.slice(-119), now]);
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
    } catch {
      /* 忽略：下一轮轮询会刷新真实状态 */
    }
    setLoading(false);
  };

  if (!gpu) {
    return (
      <Panel label="GPU 频率" index={11} className="h-full">
        <span className="font-mono text-sm opacity-30">加载 GPU 数据...</span>
      </Panel>
    );
  }

  const freqs =
    gpu.available_freqs_mhz.length > 0
      ? gpu.available_freqs_mhz
      : [257, 342, 414, 520, 596, 675, 710];
  const hwPeak = Math.max(...freqs);
  const isIdleMin = gpu.cur_freq_mhz <= gpu.min_freq_mhz;
  const isCapped = gpu.max_freq_mhz < hwPeak;
  const usagePct =
    gpu.max_freq_mhz > 0 ? Math.min((gpu.cur_freq_mhz / gpu.max_freq_mhz) * 100, 100) : 0;

  return (
    <Panel
      label="GPU 频率"
      index={11}
      className="h-full"
      hint={
        <Chip
          size="sm"
          color={isCapped ? 'warning' : isIdleMin ? 'default' : 'accent'}
          variant="secondary"
          className="font-mono text-[10px]"
        >
          {isCapped ? '已限频' : isIdleMin ? '空闲' : '运行中'} · {gpu.governor}
        </Chip>
      }
      bodyClassName="gap-3"
    >
      <div className="grid shrink-0 grid-cols-2 gap-2.5">
        <StatCell
          label="当前频率"
          value={gpu.cur_freq_mhz}
          unit="MHz"
          color="accent"
          sub={`相对上限 ${usagePct.toFixed(0)}%`}
        />
        <StatCell
          label="频率上限"
          value={gpu.max_freq_mhz}
          unit="MHz"
          sub={`最低 ${gpu.min_freq_mhz} · 峰值 ${hwPeak}`}
        />
      </div>

      <div className="flex flex-col gap-1.5">
        <MeterRow
          label="当前 / 上限"
          ratio={usagePct}
          color="accent"
          value={`${usagePct.toFixed(0)}%`}
        />
        <MeterRow
          label="上限 / 硬件峰值"
          ratio={(gpu.max_freq_mhz / hwPeak) * 100}
          color={isCapped ? 'warning' : undefined}
          value={`${((gpu.max_freq_mhz / hwPeak) * 100).toFixed(0)}%`}
        />
      </div>

      <div className="min-h-[88px] flex-1">
        <TrendChart data={history} timestamps={timestamps} variant="gpu" unit=" MHz" height={88} />
      </div>

      <div className="flex shrink-0 flex-wrap gap-1.5">
        {freqs.map((mhz) => (
          <Button
            key={mhz}
            size="sm"
            variant={gpu.max_freq_mhz === mhz ? 'secondary' : 'ghost'}
            isDisabled={loading}
            onPress={() => handleMaxFreq(mhz)}
            className="min-w-0 px-2 font-mono text-[10px] h-7"
          >
            {mhz}
          </Button>
        ))}
      </div>

      {isIdleMin && (
        <p className="mt-auto border-t border-default-100 pt-2 text-center font-mono text-[9px] opacity-35 xl:text-[10px]">
          GPU 空闲时维持在最低档，负载升高会自动升频
        </p>
      )}
    </Panel>
  );
}

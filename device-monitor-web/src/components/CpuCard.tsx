import { Card, ProgressCircle } from '@heroui/react';
import { TrendChart } from './TrendChart';
import { percentColor } from './utils';
import type { CpuInfo } from '../types';

interface CpuCardProps {
  cpu: CpuInfo;
  history: number[];
  timestamps?: number[];
  loadAvg: number[];
}

export function CpuCard({ cpu, history, timestamps, loadAvg }: CpuCardProps) {
  const color = percentColor(cpu.overall_usage);
  return (
    <Card className="p-4 sm:p-5 flex flex-col gap-3">
      <span className="text-[10px] font-mono uppercase tracking-widest opacity-50">处理器</span>
      <div className="flex items-center gap-4 sm:gap-6">
        <div className="relative inline-flex items-center justify-center">
          <ProgressCircle
            value={cpu.overall_usage}
            size="lg"
            color={color as any}
          >
            <ProgressCircle.Track className="size-24 sm:size-28">
              <ProgressCircle.TrackCircle />
              <ProgressCircle.FillCircle />
            </ProgressCircle.Track>
          </ProgressCircle>
          <span className="absolute text-xl sm:text-2xl font-mono font-light">
            {cpu.overall_usage.toFixed(0)}%
          </span>
        </div>
        <div className="flex-1 min-w-0">
          <TrendChart data={history} timestamps={timestamps} variant="cpu" />
          <div className="mt-1.5 flex flex-wrap gap-x-4 gap-y-0.5 text-[10px] sm:text-[11px] font-mono opacity-50">
            <span>负载 {loadAvg.map(v => v.toFixed(2)).join(' / ')}</span>
            <span>{cpu.cores[0]?.frequency_mhz || 0} MHz</span>
          </div>
        </div>
      </div>
    </Card>
  );
}

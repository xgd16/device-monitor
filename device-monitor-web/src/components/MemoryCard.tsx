import { Card, ProgressCircle } from '@heroui/react';
import { TrendChart } from './TrendChart';
import { percentColor } from './utils';
import type { MemoryInfo } from '../types';

interface MemoryCardProps {
  memory: MemoryInfo;
  history: number[];
  timestamps?: number[];
}

export function MemoryCard({ memory, history, timestamps }: MemoryCardProps) {
  const color = percentColor(memory.usage_percent);
  return (
    <Card className="p-4 sm:p-5 flex flex-col gap-3">
      <span className="text-[10px] font-mono uppercase tracking-widest opacity-50">内存</span>
      <div className="flex items-center gap-4 sm:gap-6">
        <div className="relative inline-flex items-center justify-center">
          <ProgressCircle
            value={memory.usage_percent}
            size="lg"
            color={color as any}
          >
            <ProgressCircle.Track className="size-24 sm:size-28">
              <ProgressCircle.TrackCircle />
              <ProgressCircle.FillCircle />
            </ProgressCircle.Track>
          </ProgressCircle>
          <span className="absolute text-xl sm:text-2xl font-mono font-light">
            {memory.usage_percent.toFixed(0)}%
          </span>
        </div>
        <div className="flex-1 min-w-0">
          <TrendChart data={history} timestamps={timestamps} variant="mem" />
          <div className="mt-1.5 text-[10px] sm:text-[11px] font-mono opacity-50">
            {(memory.used_mb / 1024).toFixed(1)} / {(memory.total_mb / 1024).toFixed(1)} GB
            {memory.swap_total_mb > 0 && (
              <span className="ml-2">Swap {(memory.swap_used_mb / 1024).toFixed(1)} / {(memory.swap_total_mb / 1024).toFixed(1)} GB</span>
            )}
          </div>
        </div>
      </div>
    </Card>
  );
}

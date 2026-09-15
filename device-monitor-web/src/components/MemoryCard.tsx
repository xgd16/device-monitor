import { TrendMetricCard } from './TrendMetricCard';
import type { MemoryInfo } from '../types';

interface MemoryCardProps {
  memory: MemoryInfo;
  history: number[];
  timestamps?: number[];
}

export function MemoryCard({ memory, history, timestamps }: MemoryCardProps) {
  const usedGb = (memory.used_mb / 1024).toFixed(1);
  const totalGb = (memory.total_mb / 1024).toFixed(1);
  const availGb = (memory.available_mb / 1024).toFixed(1);
  const freeGb = (memory.free_mb / 1024).toFixed(1);

  return (
    <TrendMetricCard
      title="内存"
      value={memory.usage_percent}
      variant="mem"
      history={history}
      timestamps={timestamps}
      index={1}
      headerExtra={
        <span className="truncate">
          可用 {availGb} GB · 空闲 {freeGb} GB
        </span>
      }
      footer={
        <>
          <span>
            已用 {usedGb} / {totalGb} GB
          </span>
          {memory.swap_total_mb > 0 && (
            <>
              <span>·</span>
              <span>
                Swap {(memory.swap_used_mb / 1024).toFixed(1)} /{' '}
                {(memory.swap_total_mb / 1024).toFixed(1)} GB
              </span>
            </>
          )}
        </>
      }
    />
  );
}

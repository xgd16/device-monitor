import { Card, ProgressCircle } from '@heroui/react';
import { TrendChart } from './TrendChart';
import { percentColor } from './utils';

interface TrendMetricCardProps {
  title: string;
  value: number;
  variant: 'cpu' | 'mem';
  history: number[];
  timestamps?: number[];
  headerExtra?: React.ReactNode;
  footer: React.ReactNode;
  banner?: React.ReactNode;
}

export function TrendMetricCard({
  title,
  value,
  variant,
  history,
  timestamps,
  headerExtra,
  footer,
  banner,
}: TrendMetricCardProps) {
  const color = percentColor(value);

  return (
    <Card className="p-4 sm:p-5 flex flex-col gap-3 h-full">
      <div className="flex items-center justify-between gap-2 min-h-7">
        <span className="text-[10px] font-mono uppercase tracking-widest opacity-50 shrink-0">{title}</span>
        {headerExtra && <div className="flex items-center gap-1.5 min-w-0">{headerExtra}</div>}
      </div>

      {banner}

      <div className="flex items-stretch gap-4 sm:gap-5 flex-1 min-h-0">
        <div className="relative inline-flex items-center justify-center shrink-0 self-center">
          <ProgressCircle value={value} size="lg" color={color as any}>
            <ProgressCircle.Track className="size-24 sm:size-28">
              <ProgressCircle.TrackCircle />
              <ProgressCircle.FillCircle />
            </ProgressCircle.Track>
          </ProgressCircle>
          <span className="absolute text-xl sm:text-2xl font-mono font-light">
            {value.toFixed(0)}%
          </span>
        </div>
        <div className="flex-1 min-w-0 flex flex-col justify-center gap-1.5">
          <TrendChart data={history} timestamps={timestamps} variant={variant} height={88} />
          <div className="text-[10px] sm:text-[11px] font-mono opacity-50 leading-relaxed">{footer}</div>
        </div>
      </div>
    </Card>
  );
}

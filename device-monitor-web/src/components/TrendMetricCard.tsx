import type { ReactNode } from 'react';
import { ProgressCircle } from '@heroui/react';
import { Panel } from './Panel';
import { TrendChart } from './TrendChart';
import { percentColor } from './utils';

interface TrendMetricCardProps {
  title: string;
  value: number;
  variant: 'cpu' | 'mem';
  history: number[];
  timestamps?: number[];
  headerExtra?: ReactNode;
  footer: ReactNode;
  banner?: ReactNode;
  index?: number;
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
  index = 0,
}: TrendMetricCardProps) {
  const color = percentColor(value);

  return (
    <Panel label={title} hint={headerExtra} index={index} bodyClassName="gap-3">
      {banner}

      <div className="flex flex-1 min-h-0 items-center gap-5 xl:gap-7">
        <div className="relative inline-flex shrink-0 items-center justify-center">
          <ProgressCircle value={value} size="lg" color={color as never}>
            <ProgressCircle.Track className="size-28 xl:size-32">
              <ProgressCircle.TrackCircle />
              <ProgressCircle.FillCircle />
            </ProgressCircle.Track>
          </ProgressCircle>
          <span className="dm-hero dm-hero--etch absolute text-2xl font-light xl:text-[2rem]">
            {value.toFixed(0)}
            <span className="text-[0.5em] opacity-60">%</span>
          </span>
        </div>

        <div className="flex min-h-0 min-w-0 flex-1 flex-col justify-center">
          <TrendChart data={history} timestamps={timestamps} variant={variant} height={124} />
        </div>
      </div>

      <div className="dm-row mt-auto flex flex-wrap items-center justify-center gap-x-2 gap-y-0.5 border-t border-b-0 pt-2 text-[10px] font-mono opacity-55 xl:text-[11px]">
        {footer}
      </div>
    </Panel>
  );
}

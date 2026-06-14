import { Card, ProgressBar } from '@heroui/react';
import { tempColor } from './utils';
import type { ThermalZone } from '../types';

interface ThermalCardProps {
  thermal: ThermalZone[];
}

export function ThermalCard({ thermal }: ThermalCardProps) {
  const sorted = [...thermal].sort((a, b) => b.temp_celsius - a.temp_celsius);
  return (
    <Card className="p-4 sm:p-5 flex flex-col gap-3">
      <span className="text-[10px] font-mono uppercase tracking-widest opacity-50">温度传感器</span>
      {sorted.length === 0 ? (
        <p className="text-xs font-mono opacity-30">未检测到传感器</p>
      ) : (
        <div className="flex flex-col gap-1.5 flex-1 overflow-y-auto min-h-0">
          {sorted.map(z => (
            <div key={z.id} className="flex items-center gap-2 text-xs py-0.5">
              <span className="flex-1 text-[10px] sm:text-[11px] truncate opacity-50">{z.name}</span>
              <div className="w-16 sm:w-20">
                <ProgressBar
                  value={z.temp_celsius}
                  maxValue={85}
                  size="sm"
                  color={tempColor(z.temp_celsius) as any}
                >
                  <ProgressBar.Track>
                    <ProgressBar.Fill />
                  </ProgressBar.Track>
                </ProgressBar>
              </div>
              <span
                className="w-10 font-mono text-[10px] sm:text-[11px] text-right"
                style={{ color: `var(--${tempColor(z.temp_celsius)})` }}
              >
                {z.temp_celsius.toFixed(1)}°
              </span>
            </div>
          ))}
        </div>
      )}
    </Card>
  );
}

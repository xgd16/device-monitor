import { Card, ProgressBar } from '@heroui/react';
import { tempColor, thermalSensorLabel } from './utils';
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
        <div className="flex flex-col gap-2 flex-1 overflow-y-auto min-h-0 pr-1">
          {sorted.map(z => {
            const label = thermalSensorLabel(z.name);
            return (
            <div key={z.id} className="flex items-center gap-2 text-xs py-0.5">
              <div className="flex-1 min-w-0">
                <div className="text-[11px] sm:text-xs truncate text-foreground/70">{label.title}</div>
                <div className="text-[9px] sm:text-[10px] truncate font-mono opacity-35">
                  {label.description} · {z.name}
                </div>
              </div>
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
            );
          })}
        </div>
      )}
    </Card>
  );
}

import { MeterRow, Panel, StatCell } from './Panel';
import { tempColor, thermalSensorLabel } from './utils';
import type { ThermalZone } from '../types';

interface ThermalCardProps {
  thermal: ThermalZone[];
  className?: string;
}

export function ThermalCard({ thermal, className }: ThermalCardProps) {
  const sorted = [...thermal].sort((a, b) => b.temp_celsius - a.temp_celsius);
  const max = sorted[0]?.temp_celsius ?? 0;
  const avg =
    sorted.length > 0 ? sorted.reduce((s, z) => s + z.temp_celsius, 0) / sorted.length : 0;
  const hot = sorted.filter((z) => z.temp_celsius >= 60).length;

  return (
    <Panel
      label="温度传感器"
      index={7}
      className={`h-full ${className ?? ''}`}
      hint={<span>{sorted.length} 个</span>}
      bodyClassName="gap-3"
    >
      {sorted.length === 0 ? (
        <p className="font-mono text-xs opacity-30">未检测到传感器</p>
      ) : (
        <>
          <div className="grid shrink-0 grid-cols-2 gap-2.5">
            <StatCell
              label="最高"
              value={max.toFixed(1)}
              unit="°C"
              color={tempColor(max)}
              sub={sorted[0].name.replace(/-thermal$/i, '')}
            />
            <StatCell
              label="平均"
              value={avg.toFixed(1)}
              unit="°C"
              color={tempColor(avg)}
              sub={hot > 0 ? `${hot} 个超 60°C` : '全部正常'}
            />
          </div>

          <div className="flex min-h-0 flex-1 flex-col gap-1.5 overflow-y-auto pr-1">
            {sorted.map((z) => {
              const label = thermalSensorLabel(z.name);
              return (
                <MeterRow
                  key={z.id}
                  label={label.title}
                  title={`${label.description} · ${z.name}`}
                  ratio={(z.temp_celsius / 85) * 100}
                  color={tempColor(z.temp_celsius)}
                  value={`${z.temp_celsius.toFixed(1)}°`}
                />
              );
            })}
          </div>
        </>
      )}
    </Panel>
  );
}

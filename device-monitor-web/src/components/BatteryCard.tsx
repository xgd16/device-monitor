import { Card, ProgressBar } from '@heroui/react';
import { batteryStatusLabel, percentColor } from './utils';
import type { BatteryInfo } from '../types';

interface BatteryCardProps {
  battery: BatteryInfo;
}

function fmtTime(mins: number): string {
  const abs = Math.abs(mins);
  const h = Math.floor(abs / 60);
  const m = abs % 60;
  if (h > 0) return `${h}时${m > 0 ? m + '分' : ''}`;
  return `${m}分`;
}

function batteryMeta(battery: BatteryInfo): { powerLabel: string; timeText: string } {
  const status = battery.status;
  const w = (battery.power_w ?? (battery.voltage_v * Math.abs(battery.current_ma) / 1000)).toFixed(1);

  let powerLabel = `${w} W`;
  if (status === 'Charging') powerLabel = `+${w} W 充电`;
  else if (status === 'Discharging') powerLabel = `-${w} W 消耗`;
  else if (status === 'Not charging') powerLabel = `${w} W 待机`;
  else if (status === 'Full') powerLabel = '已充满';

  let timeText = '';
  if (status === 'Discharging' && battery.time_left_min > 0) {
    timeText = `剩余 ${fmtTime(battery.time_left_min)}`;
  } else if (status === 'Charging' && battery.time_left_min < 0) {
    timeText = `充满 ${fmtTime(battery.time_left_min)}`;
  }

  return { powerLabel, timeText };
}

export function BatteryCard({ battery }: BatteryCardProps) {
  const color = percentColor(100 - battery.capacity);
  const statusText = batteryStatusLabel(battery.status);
  const { powerLabel, timeText } = batteryMeta(battery);
  const statusColor =
    battery.status === 'Charging'
      ? 'text-accent'
      : battery.status === 'Full'
        ? 'text-success'
        : battery.capacity < 20
          ? 'text-danger'
          : 'opacity-60';

  return (
    <Card className="p-3 sm:p-4 flex flex-col gap-2">
      <span className="text-[9px] sm:text-[10px] font-mono uppercase tracking-widest opacity-50">电池</span>

      <div className="flex items-baseline gap-2">
        <span
          className="font-mono text-xl sm:text-2xl lg:text-3xl font-light leading-none"
          style={{ color: `var(--${color})` }}
        >
          {battery.capacity}
          <span className="text-[10px] opacity-50">%</span>
        </span>
        <span className={`text-[9px] sm:text-[10px] font-mono ${statusColor}`}>{statusText}</span>
      </div>

      <ProgressBar value={battery.capacity} size="sm" color={color as any}>
        <ProgressBar.Track>
          <ProgressBar.Fill />
        </ProgressBar.Track>
      </ProgressBar>

      <div className="flex flex-col gap-0.5">
        <span className="text-[9px] sm:text-[10px] font-mono opacity-50">{powerLabel}</span>
        <div className="flex flex-wrap gap-x-3 gap-y-0.5 text-[9px] sm:text-[10px] font-mono opacity-40">
          <span>{battery.voltage_v.toFixed(2)} V</span>
          <span>{Math.abs(battery.current_ma).toFixed(0)} mA</span>
          {battery.temp_celsius > 0 && <span>{battery.temp_celsius.toFixed(1)} °C</span>}
        </div>
      </div>

      {timeText && <span className="text-[9px] font-mono opacity-25">{timeText}</span>}
    </Card>
  );
}

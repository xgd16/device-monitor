import { Card, ProgressBar, Chip } from '@heroui/react';
import { percentColor } from './utils';
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

function fmtPower(battery: BatteryInfo): { label: string; watts: string; color: string } {
  const w = battery.power_w ?? (battery.voltage_v * Math.abs(battery.current_ma) / 1000);
  const watts = w.toFixed(1);

  if (battery.status === 'Charging') {
    return { label: '充电功率', watts: `${watts} W`, color: 'var(--accent)' };
  }
  if (battery.status === 'Discharging') {
    return { label: '消耗功率', watts: `${watts} W`, color: 'var(--warning)' };
  }
  return { label: '功率', watts: `${watts} W`, color: 'var(--default)' };
}

export function BatteryCard({ battery }: BatteryCardProps) {
  const isCharging = battery.status === 'Charging';
  const color = percentColor(100 - battery.capacity);
  const power = fmtPower(battery);

  let statusText = isCharging ? '充电中' : '放电中';
  let timeText = '';
  if (battery.time_left_min > 0) {
    statusText = '放电中';
    timeText = `剩余 ${fmtTime(battery.time_left_min)}`;
  } else if (battery.time_left_min < 0) {
    statusText = '充电中';
    timeText = `充满 ${fmtTime(battery.time_left_min)}`;
  }

  return (
    <Card className="p-4 sm:p-5 flex flex-col gap-3">
      <div className="flex items-center justify-between">
        <span className="text-[10px] font-mono uppercase tracking-widest opacity-50">电池</span>
        <Chip
          size="sm"
          color={isCharging ? 'accent' : battery.capacity < 20 ? 'danger' : 'default'}
          variant="secondary"
        >
          {statusText}
        </Chip>
      </div>

      <div className="flex items-center gap-3">
        <div className="flex-1">
          <ProgressBar
            value={battery.capacity}
            size="md"
            color={color as any}
          >
            <ProgressBar.Track>
              <ProgressBar.Fill />
            </ProgressBar.Track>
          </ProgressBar>
        </div>
        <span className="font-mono text-xl sm:text-2xl font-light w-14 text-right" style={{ color: `var(--${color})` }}>
          {battery.capacity}<span className="text-[10px] opacity-50">%</span>
        </span>
      </div>

      {/* 充电/消耗瓦数 */}
      <div className="flex items-baseline gap-2">
        <span className="text-[10px] sm:text-[11px] font-mono opacity-50">{power.label}</span>
        <span className="font-mono text-lg sm:text-xl font-medium" style={{ color: power.color }}>
          {power.watts}
        </span>
      </div>

      <div className="flex flex-wrap gap-x-4 gap-y-1 font-mono text-[10px] sm:text-[11px] opacity-50">
        <span>电压 {battery.voltage_v.toFixed(2)} V</span>
        <span>电流 {Math.abs(battery.current_ma).toFixed(0)} mA</span>
        {battery.temp_celsius > 0 && <span>温度 {battery.temp_celsius.toFixed(1)} °C</span>}
        {timeText && <span className="opacity-70">{timeText}</span>}
      </div>
    </Card>
  );
}

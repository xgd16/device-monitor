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

export function BatteryCard({ battery }: BatteryCardProps) {
  const isCharging = battery.status === 'Charging';
  const color = percentColor(100 - battery.capacity); // 电量低=红

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

      {/* 进度条 + 百分比 */}
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

      {/* 详细信息 */}
      <div className="flex flex-wrap gap-x-4 gap-y-1 font-mono text-[10px] sm:text-[11px] opacity-50">
        <span>电压 {battery.voltage_v.toFixed(2)}V</span>
        <span>电流 {battery.current_ma.toFixed(0)}mA</span>
        {battery.temp_celsius > 0 && <span>温度 {battery.temp_celsius.toFixed(1)}°C</span>}
        {timeText && <span className="opacity-70">{timeText}</span>}
      </div>
    </Card>
  );
}

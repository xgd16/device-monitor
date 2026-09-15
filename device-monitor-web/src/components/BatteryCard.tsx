import { Hero, MeterRow, Panel } from './Panel';
import {
  batteryStatusLabel,
  batteryDisplayCapacity,
  batteryCapacityHint,
  percentColor,
} from './utils';
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
  const w = (battery.power_w ?? (battery.voltage_v * Math.abs(battery.current_ma)) / 1000).toFixed(
    1,
  );

  let powerLabel = `${w} W`;
  if (status === 'Charging') powerLabel = `+${w} W 充电`;
  else if (status === 'Discharging') powerLabel = `-${w} W 消耗`;
  else if (status === 'Not charging') powerLabel = `${w} W 待机`;
  else if (status === 'Full') {
    powerLabel = battery.at_charge_limit && battery.is_degraded ? '已达实际上限' : '已充满';
  }

  let timeText = '';
  if (status === 'Discharging' && battery.time_left_min > 0) {
    timeText = `剩余 ${fmtTime(battery.time_left_min)}`;
  } else if (status === 'Charging' && battery.time_left_min < 0 && !battery.at_charge_limit) {
    const target = battery.is_degraded ? `上限 ${battery.effective_max_pct}%` : '100%';
    timeText = `至 ${target} ${fmtTime(battery.time_left_min)}`;
  }

  return { powerLabel, timeText };
}

export function BatteryCard({ battery }: BatteryCardProps) {
  const displayPct = batteryDisplayCapacity(battery);
  const color = percentColor(100 - displayPct);
  const statusText = batteryStatusLabel(battery.status, battery);
  const capacityHint = batteryCapacityHint(battery);
  const { powerLabel, timeText } = batteryMeta(battery);

  const statusTone =
    battery.status === 'Charging'
      ? 'text-accent'
      : battery.status === 'Full' || battery.at_charge_limit
        ? 'text-success'
        : displayPct < 20
          ? 'text-danger'
          : 'opacity-55';

  return (
    <Panel
      label="电池"
      index={5}
      hint={
        <>
          {capacityHint && <span>{capacityHint}</span>}
          <span className={statusTone}>{statusText}</span>
        </>
      }
    >
      <Hero
        value={battery.capacity}
        unit="%"
        color={color}
        note={
          battery.is_degraded && displayPct !== battery.capacity ? `相对 ${displayPct}%` : undefined
        }
      />

      <div className="flex flex-col gap-1.5">
        <MeterRow
          label="电压"
          ratio={(battery.voltage_v / 4.4) * 100}
          value={`${battery.voltage_v.toFixed(2)}V`}
        />
        <MeterRow
          label="电流"
          ratio={(Math.abs(battery.current_ma) / 3600) * 100}
          value={`${Math.abs(battery.current_ma).toFixed(0)}mA`}
        />
        <MeterRow
          label="温度"
          ratio={(battery.temp_celsius / 50) * 100}
          value={`${battery.temp_celsius.toFixed(1)}°`}
        />
      </div>

      <div className="mt-auto flex flex-wrap items-center justify-center gap-x-2 border-t border-default-100 pt-2 font-mono text-[9px] opacity-40 xl:text-[10px]">
        <span>{powerLabel}</span>
        {timeText && (
          <>
            <span>·</span>
            <span>{timeText}</span>
          </>
        )}
      </div>
    </Panel>
  );
}

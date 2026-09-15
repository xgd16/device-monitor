import { Chip } from '@heroui/react';
import { Panel, StatCell } from './Panel';
import type { WifiInfo, BluetoothInfo } from '../types';

interface WirelessCardProps {
  wifi: WifiInfo | null;
  bluetooth: BluetoothInfo | null;
}

/** 信号强度：-30 dBm 满格 → -90 dBm 空 */
function signalRatio(dbm: number): number {
  return ((Math.max(-90, Math.min(-30, dbm)) + 90) / 60) * 100;
}

export function WirelessCard({ wifi, bluetooth }: WirelessCardProps) {
  const paired = bluetooth?.devices.filter((d) => d.connected).length ?? 0;

  return (
    <Panel
      label="无线连接"
      index={9}
      className="h-full"
      hint={bluetooth ? <span>{bluetooth.devices.length} 个已配对</span> : undefined}
      bodyClassName="gap-3"
    >
      <div className="grid shrink-0 grid-cols-2 gap-2.5">
        <StatCell
          label="WiFi 信号"
          value={wifi?.connected ? wifi.signal_dbm : '—'}
          unit={wifi?.connected ? 'dBm' : undefined}
          color={
            wifi?.connected
              ? signalRatio(wifi.signal_dbm) > 55
                ? 'success'
                : signalRatio(wifi.signal_dbm) > 25
                  ? 'warning'
                  : 'danger'
              : undefined
          }
          sub={wifi?.connected ? `${signalRatio(wifi.signal_dbm).toFixed(0)}% 强度` : '未连接'}
        />
        <StatCell
          label="蓝牙"
          value={bluetooth?.powered ? '开启' : '关闭'}
          color={bluetooth?.powered ? 'accent' : undefined}
          sub={`${paired} 个已连接`}
        />
      </div>

      <div className="flex flex-col gap-2.5">
        <div className="dm-inset flex flex-col gap-1.5 p-2.5">
          <div className="flex items-center gap-2">
            <span
              className={
                wifi?.connected ? 'dm-live' : 'inline-block size-[7px] rounded-full bg-default-300'
              }
            />
            <span className="font-mono text-xs font-medium xl:text-sm">WiFi</span>
            {wifi?.connected && (
              <Chip size="sm" color="accent" variant="secondary" className="ml-auto">
                {wifi.ssid}
              </Chip>
            )}
          </div>
          {wifi?.connected ? (
            <div className="flex flex-col gap-0.5 font-mono text-[10px] opacity-50 xl:text-[11px]">
              <span>
                {wifi.band} · Ch{wifi.channel} · {wifi.frequency_mhz} MHz
              </span>
              <span>速率 {wifi.bitrate}</span>
              <span className="truncate opacity-70">BSSID {wifi.bssid}</span>
            </div>
          ) : (
            <span className="font-mono text-[10px] opacity-30">未连接</span>
          )}
        </div>

        <div className="dm-inset flex flex-col gap-1.5 p-2.5">
          <div className="flex items-center gap-2">
            <span
              className={
                bluetooth?.powered
                  ? 'dm-live'
                  : 'inline-block size-[7px] rounded-full bg-default-300'
              }
            />
            <span className="font-mono text-xs font-medium xl:text-sm">蓝牙</span>
            <Chip
              size="sm"
              color={bluetooth?.powered ? 'accent' : 'default'}
              variant="secondary"
              className="ml-auto"
            >
              {bluetooth?.powered ? '已开启' : '已关闭'}
            </Chip>
          </div>
          {bluetooth?.powered && (
            <div className="flex flex-col gap-0.5 font-mono text-[10px] opacity-50 xl:text-[11px]">
              <span>{bluetooth.name || bluetooth.address || '已激活'}</span>
              {bluetooth.devices.length > 0 && (
                <span className="truncate">
                  {bluetooth.devices
                    .slice(0, 3)
                    .map((d) => d.name || d.address)
                    .join(' · ')}
                </span>
              )}
            </div>
          )}
        </div>
      </div>
    </Panel>
  );
}

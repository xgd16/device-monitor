import { Card, Chip } from '@heroui/react';
import type { WifiInfo, BluetoothInfo } from '../types';

interface WirelessCardProps {
  wifi: WifiInfo | null;
  bluetooth: BluetoothInfo | null;
}

export function WirelessCard({ wifi, bluetooth }: WirelessCardProps) {
  return (
    <Card className="p-4 sm:p-5 flex flex-col gap-4">
      <span className="text-[10px] font-mono uppercase tracking-widest opacity-50">无线连接</span>

      {/* WiFi */}
      <div>
        <div className="flex items-center gap-2 mb-1">
          <span
            className="inline-block w-[6px] h-[6px] rounded-full flex-shrink-0"
            style={{ background: wifi?.connected ? 'var(--success)' : 'var(--default)' }}
          />
          <span className="font-mono font-medium text-xs sm:text-sm">WiFi</span>
          {wifi?.connected && <Chip size="sm" color="success" variant="secondary">{wifi.ssid}</Chip>}
        </div>
        {wifi?.connected ? (
          <div className="pl-4 font-mono text-[10px] sm:text-[11px] leading-relaxed opacity-50">
            <div>信号 {wifi.signal_dbm} dBm · {wifi.band} Ch{wifi.channel} ({wifi.frequency_mhz} MHz)</div>
            <div>速率 {wifi.bitrate}</div>
            <div className="hidden sm:block">BSSID {wifi.bssid}</div>
          </div>
        ) : (
          <p className="pl-4 text-[10px] sm:text-[11px] font-mono opacity-30">未连接</p>
        )}
      </div>

      {/* Bluetooth */}
      <div>
        <div className="flex items-center gap-2 mb-1">
          <span
            className="inline-block w-[6px] h-[6px] rounded-full flex-shrink-0"
            style={{ background: bluetooth?.powered ? 'var(--accent)' : 'var(--default)' }}
          />
          <span className="font-mono font-medium text-xs sm:text-sm">蓝牙</span>
          <Chip size="sm" color={bluetooth?.powered ? 'accent' : 'default'} variant="secondary">
            {bluetooth?.powered ? '已开启' : '已关闭'}
          </Chip>
        </div>
        {bluetooth?.powered && (
          <div className="pl-4 font-mono text-[10px] sm:text-[11px] opacity-50">
            <div>{bluetooth.name || bluetooth.address || '已激活'}</div>
            <div>{bluetooth.devices.length} 个已配对设备</div>
          </div>
        )}
      </div>
    </Card>
  );
}

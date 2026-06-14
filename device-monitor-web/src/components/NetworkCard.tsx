import { Card } from '@heroui/react';
import { fmtBytes, fmtSpeed } from './utils';
import type { NetworkInterface } from '../types';

interface NetworkCardProps {
  network: NetworkInterface[];
  netSpeed: Record<string, { rx: number; tx: number }>;
}

export function NetworkCard({ network, netSpeed }: NetworkCardProps) {
  if (network.length === 0) {
    return (
      <Card className="p-4 sm:p-5 flex flex-col gap-3">
        <span className="text-[10px] font-mono uppercase tracking-widest opacity-50">网络接口</span>
        <p className="text-xs font-mono opacity-30">无网络接口</p>
      </Card>
    );
  }

  return (
    <Card className="p-4 sm:p-5 flex flex-col gap-3">
      <span className="text-[10px] font-mono uppercase tracking-widest opacity-50">
        网络接口 <span className="opacity-40">({network.length})</span>
      </span>
      <div className="flex flex-col gap-2.5">
        {network.map(n => {
          const speed = netSpeed[n.name] || { rx: 0, tx: 0 };
          return (
            <div key={n.name} className="py-1.5 border-b border-default-200 last:border-0">
              <div className="flex items-center gap-2 mb-1">
                <span
                  className="inline-block w-[6px] h-[6px] rounded-full flex-shrink-0"
                  style={{ background: n.is_up ? 'var(--success)' : 'var(--default)' }}
                />
                <span className="font-mono text-xs sm:text-sm font-medium">{n.name}</span>
                {!n.is_up && (
                  <span className="font-mono text-[9px] px-1.5 py-0.5 rounded bg-default-100 opacity-40">down</span>
                )}
              </div>
              {n.ip_addresses.length > 0 && (
                <div className="pl-4 mb-0.5">
                  {n.ip_addresses.map((ip, i) => (
                    <div key={i} className="font-mono text-[10px] sm:text-[11px] opacity-40">{ip}</div>
                  ))}
                </div>
              )}
              <div className="flex flex-wrap gap-x-4 gap-y-0.5 font-mono text-[10px] sm:text-[11px] pl-4">
                <span className="text-primary">↓ {speed.rx > 0 ? fmtSpeed(speed.rx) : '0 B/s'}</span>
                <span className="text-warning">↑ {speed.tx > 0 ? fmtSpeed(speed.tx) : '0 B/s'}</span>
                <span className="opacity-30">累计 ↓{fmtBytes(n.rx_bytes)} ↑{fmtBytes(n.tx_bytes)}</span>
                <span className="opacity-30">包 ↓{n.rx_packets.toLocaleString()} ↑{n.tx_packets.toLocaleString()}</span>
              </div>
            </div>
          );
        })}
      </div>
    </Card>
  );
}

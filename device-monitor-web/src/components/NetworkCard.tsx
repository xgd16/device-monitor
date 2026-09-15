import { Panel } from './Panel';
import { fmtBytes, fmtSpeed } from './utils';
import type { NetworkInterface } from '../types';

interface NetworkCardProps {
  network: NetworkInterface[];
  netSpeed: Record<string, { rx: number; tx: number }>;
}

export function NetworkCard({ network, netSpeed }: NetworkCardProps) {
  const upCount = network.filter((n) => n.is_up).length;
  const totalRx = network.reduce((s, n) => s + (netSpeed[n.name]?.rx ?? 0), 0);
  const totalTx = network.reduce((s, n) => s + (netSpeed[n.name]?.tx ?? 0), 0);

  return (
    <Panel
      label="网络接口"
      index={8}
      className="h-full"
      hint={
        <>
          <span className="text-accent">↓ {fmtSpeed(totalRx)}</span>
          <span className="text-warning">↑ {fmtSpeed(totalTx)}</span>
        </>
      }
      bodyClassName="gap-0"
    >
      {network.length === 0 ? (
        <p className="font-mono text-xs opacity-30">无网络接口</p>
      ) : (
        <div className="flex flex-col">
          {network.map((n) => {
            const speed = netSpeed[n.name] || { rx: 0, tx: 0 };
            return (
              <div
                key={n.name}
                className="dm-row flex flex-col gap-1.5 py-2.5 first:pt-0 last:pb-0"
              >
                <div className="flex items-center gap-2">
                  {n.is_up ? (
                    <span className="dm-live" />
                  ) : (
                    <span className="inline-block size-[7px] shrink-0 rounded-full bg-default-300" />
                  )}
                  <span className="font-mono text-xs font-medium xl:text-sm">{n.name}</span>
                  {!n.is_up && (
                    <span className="rounded bg-default-100 px-1.5 py-0.5 font-mono text-[9px] opacity-50">
                      down
                    </span>
                  )}
                  <span className="ml-auto flex shrink-0 items-baseline gap-2 font-mono text-[10px]">
                    <span className="text-accent">↓ {fmtSpeed(speed.rx)}</span>
                    <span className="text-warning">↑ {fmtSpeed(speed.tx)}</span>
                  </span>
                </div>

                {n.ip_addresses.length > 0 && (
                  <div className="flex flex-col gap-0.5 pl-4">
                    {n.ip_addresses.map((ip, i) => (
                      <span
                        key={i}
                        className="truncate font-mono text-[10px] opacity-45 xl:text-[11px]"
                      >
                        {ip}
                      </span>
                    ))}
                  </div>
                )}

                <div className="flex flex-wrap gap-x-4 gap-y-0.5 pl-4 font-mono text-[10px] opacity-35 xl:text-[11px]">
                  <span>
                    累计 ↓{fmtBytes(n.rx_bytes)} ↑{fmtBytes(n.tx_bytes)}
                  </span>
                  <span>
                    包 ↓{n.rx_packets.toLocaleString()} ↑{n.tx_packets.toLocaleString()}
                  </span>
                </div>
              </div>
            );
          })}
        </div>
      )}

      {network.length > 0 && (
        <div className="mt-auto flex flex-wrap items-center justify-center gap-x-2 border-t border-default-100 pt-2 font-mono text-[9px] opacity-40 xl:text-[10px]">
          <span>{network.length} 个接口</span>
          <span>·</span>
          <span>{upCount} 个在线</span>
        </div>
      )}
    </Panel>
  );
}

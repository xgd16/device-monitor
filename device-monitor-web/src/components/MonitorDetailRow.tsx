import { useEffect, useRef, useState } from 'react';
import { CoreBars } from './CoreBars';
import { ThermalCard } from './ThermalCard';
import { NetworkCard } from './NetworkCard';
import { WirelessCard } from './WirelessCard';
import type { CpuCore, ThermalZone, NetworkInterface, WifiInfo, BluetoothInfo } from '../types';

interface MonitorDetailRowProps {
  cores: CpuCore[];
  overallUsage: number;
  loadAvg: number[];
  thermal: ThermalZone[];
  network: NetworkInterface[];
  netSpeed: Record<string, { rx: number; tx: number }>;
  wifi: WifiInfo | null;
  bluetooth: BluetoothInfo | null;
}

export function MonitorDetailRow({
  cores,
  overallUsage,
  loadAvg,
  thermal,
  network,
  netSpeed,
  wifi,
  bluetooth,
}: MonitorDetailRowProps) {
  const rightRef = useRef<HTMLDivElement>(null);
  const [rightHeight, setRightHeight] = useState<number>();

  useEffect(() => {
    const el = rightRef.current;
    if (!el) return;

    const sync = () => setRightHeight(el.offsetHeight);
    sync();

    const ro = new ResizeObserver(sync);
    ro.observe(el);
    return () => ro.disconnect();
  }, [network, wifi, bluetooth, cores.length, thermal.length]);

  const matchedHeight =
    rightHeight !== undefined ? { height: rightHeight } : undefined;

  return (
    <div className="grid grid-cols-3 gap-3 items-start">
      <div className="min-h-0 overflow-hidden" style={matchedHeight}>
        <CoreBars
          cores={cores}
          overallUsage={overallUsage}
          loadAvg={loadAvg}
          thermal={thermal}
        />
      </div>
      <div className="min-h-0 overflow-hidden" style={matchedHeight}>
        <ThermalCard thermal={thermal} />
      </div>
      <div ref={rightRef} className="flex flex-col gap-3">
        <NetworkCard network={network} netSpeed={netSpeed} />
        <WirelessCard wifi={wifi} bluetooth={bluetooth} />
      </div>
    </div>
  );
}

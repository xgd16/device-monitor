import { useEffect, useState, useMemo, useRef } from 'react';
import { Tabs, Spinner } from '@heroui/react';
import { useDeviceStore } from './stores/useDeviceStore';
import { useWebSocket } from './hooks/useWebSocket';
import {
  fetchProcesses,
  fetchWifi,
  fetchBluetooth,
  fetchAlerts,
  fetchRefresh,
  setRefresh,
} from './api';
import type { ProcessInfo, WifiInfo, BluetoothInfo, AlertItem } from './types';
import { StatusBar, type AppPage } from './components/StatusBar';
import { Band } from './components/Panel';
import { CpuCard } from './components/CpuCard';
import { MemoryCard } from './components/MemoryCard';
import { MetricsBar } from './components/MetricsBar';
import { CoreBars } from './components/CoreBars';
import { ThermalCard } from './components/ThermalCard';
import { NetworkCard } from './components/NetworkCard';
import { WirelessCard } from './components/WirelessCard';
import { ProcessManager } from './components/ProcessManager';
import { AlertsCard } from './components/AlertsCard';
import { BatteryCard } from './components/BatteryCard';
import { DiskCard } from './components/DiskCard';
import { HardwareControl } from './components/HardwareControl';
import { GpuMonitorCard } from './components/GpuMonitorCard';
import { SystemStatusCard } from './components/SystemStatusCard';
import { ReportsPanel } from './components/ReportsPanel';
import { TerminalPanel } from './components/TerminalPanel';
import { FileManager } from './components/FileManager';

export default function App() {
  const data = useDeviceStore((s) => s.data);
  const history = useDeviceStore((s) => s.history);
  const connected = useDeviceStore((s) => s.connected);
  useWebSocket();

  const [page, setPage] = useState<AppPage>(() => {
    const saved = localStorage.getItem('dm-page');
    if (saved === 'monitor' || saved === 'terminal' || saved === 'files') return saved;
    return 'monitor';
  });

  const [theme, setTheme] = useState<'dark' | 'light'>(() => {
    const saved = localStorage.getItem('dm-theme');
    if (saved === 'light' || saved === 'dark') return saved;
    return window.matchMedia('(prefers-color-scheme: light)').matches ? 'light' : 'dark';
  });

  // 界面刷新间隔（秒）：先显示本地缓存的值避免闪烁，再以服务端为准
  const [refreshSecs, setRefreshSecs] = useState<number>(() => {
    const saved = Number(localStorage.getItem('dm-refresh'));
    return [1, 3, 5, 10].includes(saved) ? saved : 5;
  });

  useEffect(() => {
    fetchRefresh()
      .then((c) => setRefreshSecs(c.refresh_secs))
      .catch(() => {});
  }, []);

  const changeRefresh = (secs: number) => {
    setRefreshSecs(secs);
    localStorage.setItem('dm-refresh', String(secs));
    setRefresh(secs).catch(() => {
      // 失败时回滚为服务端实际值
      fetchRefresh()
        .then((c) => setRefreshSecs(c.refresh_secs))
        .catch(() => {});
    });
  };

  useEffect(() => {
    document.documentElement.setAttribute('data-theme', theme);
    localStorage.setItem('dm-theme', theme);
  }, [theme]);

  useEffect(() => {
    localStorage.setItem('dm-page', page);
  }, [page]);

  const [processes, setProcesses] = useState<ProcessInfo[]>([]);
  const [wifi, setWifi] = useState<WifiInfo | null>(null);
  const [bluetooth, setBluetooth] = useState<BluetoothInfo | null>(null);
  const [alerts, setAlerts] = useState<AlertItem[]>([]);

  const prevNetRef = useRef<Record<string, { rx: number; tx: number; ts: number }>>({});
  const netSpeed = useMemo(() => {
    if (!data) return {};
    const now = data.timestamp;
    const speeds: Record<string, { rx: number; tx: number }> = {};
    for (const iface of data.network) {
      const prev = prevNetRef.current[iface.name];
      if (prev && now > prev.ts) {
        const dt = now - prev.ts;
        speeds[iface.name] = {
          rx: Math.max(0, (iface.rx_bytes - prev.rx) / dt),
          tx: Math.max(0, (iface.tx_bytes - prev.tx) / dt),
        };
      } else {
        speeds[iface.name] = { rx: 0, tx: 0 };
      }
    }
    for (const iface of data.network) {
      prevNetRef.current[iface.name] = { rx: iface.rx_bytes, tx: iface.tx_bytes, ts: now };
    }
    return speeds;
  }, [data]);

  useEffect(() => {
    const load = () => {
      fetchProcesses()
        .then(setProcesses)
        .catch(() => {});
      fetchWifi()
        .then(setWifi)
        .catch(() => {});
      fetchBluetooth()
        .then(setBluetooth)
        .catch(() => {});
      fetchAlerts()
        .then(setAlerts)
        .catch(() => {});
    };
    load();
    const t = setInterval(load, 10000);
    return () => clearInterval(t);
  }, []);

  const cpuHistory = useMemo(() => history.map((h) => h.cpu.overall_usage), [history]);
  const memHistory = useMemo(() => history.map((h) => h.memory.usage_percent), [history]);
  const historyTimestamps = useMemo(() => history.map((h) => h.timestamp), [history]);

  if (!data) {
    return (
      <div className="flex items-center justify-center h-dvh">
        <div className="text-center flex flex-col items-center gap-4">
          <Spinner size="lg" />
          <p className="text-sm text-foreground/50">
            {connected ? '已连接，等待数据...' : '正在连接...'}
          </p>
        </div>
      </div>
    );
  }

  const { cpu, memory, network } = data;

  return (
    <div className="h-dvh flex flex-col overflow-hidden">
      <StatusBar
        connected={connected}
        uptime={data.uptime}
        theme={theme}
        page={page}
        onThemeChange={setTheme}
        onPageChange={setPage}
        refreshSecs={refreshSecs}
        onRefreshChange={changeRefresh}
      />

      {page === 'terminal' && (
        <div className="flex-1 flex flex-col min-h-0 p-3 md:p-4">
          <TerminalPanel fullPage />
        </div>
      )}

      {page === 'files' && (
        <div className="flex-1 flex flex-col min-h-0 p-3 md:p-4">
          <FileManager fullPage />
        </div>
      )}

      {page === 'monitor' && (
        <>
          {/* Mobile: Tab navigation (< md) */}
          <div className="flex-1 min-h-0 overflow-y-auto md:hidden">
            <Tabs aria-label="导航" variant="secondary" className="w-full">
              <Tabs.List className="w-full">
                <Tabs.Tab id="overview">概览</Tabs.Tab>
                <Tabs.Tab id="cpu">CPU</Tabs.Tab>
                <Tabs.Tab id="net">网络</Tabs.Tab>
                <Tabs.Tab id="hw">控制</Tabs.Tab>
                <Tabs.Tab id="more">更多</Tabs.Tab>
                <Tabs.Tab id="reports">报表</Tabs.Tab>
              </Tabs.List>

              <Tabs.Panel id="overview" className="p-3 flex flex-col gap-3">
                <CpuCard
                  cpu={cpu}
                  history={cpuHistory}
                  timestamps={historyTimestamps}
                  loadAvg={data.load_avg}
                />
                <MemoryCard memory={memory} history={memHistory} timestamps={historyTimestamps} />
                <BatteryCard battery={data.battery} />
                <MetricsBar data={data} processes={processes} />
                <AlertsCard alerts={alerts} />
              </Tabs.Panel>

              <Tabs.Panel id="cpu" className="p-3 flex flex-col gap-3">
                <CoreBars
                  cores={cpu.cores}
                  overallUsage={cpu.overall_usage}
                  loadAvg={data.load_avg}
                  thermal={data.thermal}
                />
                <ThermalCard thermal={data.thermal} />
              </Tabs.Panel>

              <Tabs.Panel id="net" className="p-3 flex flex-col gap-3">
                <NetworkCard network={network} netSpeed={netSpeed} />
                <WirelessCard wifi={wifi} bluetooth={bluetooth} />
              </Tabs.Panel>

              <Tabs.Panel id="hw" className="p-3 flex flex-col gap-3">
                <HardwareControl />
              </Tabs.Panel>

              <Tabs.Panel id="more" className="p-3 flex flex-col gap-3">
                <DiskCard />
                <ProcessManager
                  processes={processes}
                  onRefresh={() =>
                    fetchProcesses()
                      .then(setProcesses)
                      .catch(() => {})
                  }
                />
              </Tabs.Panel>

              <Tabs.Panel id="reports" className="p-3 flex flex-col gap-3">
                <ReportsPanel />
              </Tabs.Panel>
            </Tabs>
          </div>

          {/* Desktop: 中轴对称的分区版心 (>= md) */}
          <div className="hidden md:flex md:min-h-0 md:flex-1 md:flex-col md:overflow-y-auto">
            <div className="mx-auto flex w-full max-w-[1760px] flex-col gap-4 p-4 lg:gap-5 lg:p-5">
              {/* 1 · 核心指标 —— 2 等分 */}
              <Band index={1} title="核心指标" cols={2}>
                <CpuCard
                  cpu={cpu}
                  history={cpuHistory}
                  timestamps={historyTimestamps}
                  loadAvg={data.load_avg}
                />
                <MemoryCard memory={memory} history={memHistory} timestamps={historyTimestamps} />
              </Band>

              {/* 2 · 实时体征 —— 4 等分 */}
              <Band index={2} title="实时体征" cols={4}>
                <MetricsBar data={data} processes={processes} />
                <BatteryCard battery={data.battery} />
              </Band>

              {/* 3 · 深度诊断 —— 2 等分 */}
              <Band index={3} title="深度诊断" cols={2}>
                <CoreBars
                  cores={cpu.cores}
                  overallUsage={cpu.overall_usage}
                  loadAvg={data.load_avg}
                  thermal={data.thermal}
                />
                <ThermalCard thermal={data.thermal} />
              </Band>

              {/* 4 · 连接状态 —— 2 等分 */}
              <Band index={4} title="连接状态" cols={2}>
                <NetworkCard network={network} netSpeed={netSpeed} />
                <WirelessCard wifi={wifi} bluetooth={bluetooth} />
              </Band>

              {/* 5 · 存储与图形 —— 2 等分 */}
              <Band index={5} title="存储与图形" cols={2}>
                <DiskCard />
                <GpuMonitorCard />
              </Band>

              {/* 6 · 系统健康 —— 2 等分 */}
              <Band index={6} title="系统健康" cols={2}>
                <SystemStatusCard data={data} processes={processes} netSpeed={netSpeed} />
                <AlertsCard alerts={alerts} />
              </Band>

              {/* 7 · 硬件控制 —— 内部 2×4 镜像 */}
              <Band index={7} title="硬件控制" cols={1}>
                <HardwareControl />
              </Band>

              {/* 8 · 进程管理 */}
              <Band index={8} title="进程管理" cols={1}>
                <ProcessManager
                  processes={processes}
                  onRefresh={() =>
                    fetchProcesses()
                      .then(setProcesses)
                      .catch(() => {})
                  }
                />
              </Band>

              {/* 9 · 历史报表 */}
              <Band index={9} title="历史报表" cols={1}>
                <ReportsPanel />
              </Band>

              <footer className="flex items-center justify-between py-2 font-mono text-[10px] text-foreground/30">
                <span>设备监控 v0.2.0</span>
                <span>
                  最后更新{' '}
                  {new Date(data.timestamp * 1000).toLocaleTimeString('zh-CN', {
                    hour12: false,
                  })}
                </span>
              </footer>
            </div>
          </div>
        </>
      )}
    </div>
  );
}

import { Card, ProgressBar } from '@heroui/react';
import { percentColor, tempColor, thermalSensorLabel } from './utils';
import type { CpuCore, ThermalZone } from '../types';

interface CoreBarsProps {
  cores: CpuCore[];
  overallUsage: number;
  loadAvg: number[];
  thermal?: ThermalZone[];
  className?: string;
}

function calcStats(cores: CpuCore[]) {
  if (cores.length === 0) return null;
  const usages = cores.map((c) => c.usage);
  const freqs = cores.map((c) => c.frequency_mhz);
  const avg = usages.reduce((a, b) => a + b, 0) / cores.length;
  const max = Math.max(...usages);
  const min = Math.min(...usages);
  const maxCore = cores.find((c) => c.usage === max);
  const minCore = cores.find((c) => c.usage === min);
  const variance = usages.reduce((s, u) => s + (u - avg) ** 2, 0) / cores.length;
  return {
    avg,
    max,
    min,
    maxCoreId: maxCore?.id ?? 0,
    minCoreId: minCore?.id ?? 0,
    freqMin: Math.min(...freqs),
    freqMax: Math.max(...freqs),
    stddev: Math.sqrt(variance),
    busy: cores.filter((c) => c.usage > 30).length,
    idle: cores.filter((c) => c.usage < 5).length,
    totalLoad: usages.reduce((a, b) => a + b, 0),
  };
}

function balanceLabel(stddev: number): { text: string; color: string } {
  if (stddev < 10) return { text: '均衡', color: 'text-success' };
  if (stddev < 25) return { text: '轻度倾斜', color: 'text-warning' };
  return { text: '严重倾斜', color: 'text-danger' };
}

function usageBgColor(usage: number): string {
  if (usage > 80) return 'var(--danger)';
  if (usage > 50) return 'var(--warning)';
  if (usage > 20) return 'var(--accent)';
  if (usage > 5) return 'var(--default)';
  return 'transparent';
}

function coreStateLabel(usage: number): string {
  if (usage > 80) return '满载';
  if (usage > 30) return '繁忙';
  if (usage > 5) return '轻载';
  return '空闲';
}

function clusterInfo(cores: CpuCore[]): string {
  if (cores.length <= 1) return '';
  const half = Math.floor(cores.length / 2);
  const smallMaxFreq = Math.max(...cores.filter((c) => c.id < half).map((c) => c.frequency_mhz));
  const bigMaxFreq = Math.max(...cores.filter((c) => c.id >= half).map((c) => c.frequency_mhz));
  if (smallMaxFreq !== bigMaxFreq) {
    return `${half}大@${bigMaxFreq} + ${cores.length - half}小@${smallMaxFreq}`;
  }
  return `${cores.length} 核`;
}

function splitClusters(cores: CpuCore[]) {
  if (cores.length <= 1) {
    return [{ label: '全部核心', cores }];
  }

  const half = Math.floor(cores.length / 2);
  const small = cores.filter((c) => c.id < half);
  const big = cores.filter((c) => c.id >= half);
  const smallMax = Math.max(...small.map((c) => c.frequency_mhz));
  const bigMax = Math.max(...big.map((c) => c.frequency_mhz));

  if (smallMax === bigMax) {
    return [{ label: '全部核心', cores }];
  }

  return [
    { label: `小核心 C0–C${half - 1}`, cores: small },
    { label: `大核心 C${half}–C${cores.length - 1}`, cores: big },
  ];
}

function clusterStats(cores: CpuCore[]) {
  const usages = cores.map((c) => c.usage);
  const freqs = cores.map((c) => c.frequency_mhz);
  const avgUsage = usages.reduce((a, b) => a + b, 0) / cores.length;
  const avgFreq = freqs.reduce((a, b) => a + b, 0) / freqs.length;
  const active = cores.filter((c) => c.usage > 5).length;
  return { avgUsage, avgFreq, active };
}

function cpuThermalZones(thermal: ThermalZone[]) {
  return thermal
    .filter((t) => {
      const n = t.name.toLowerCase();
      return n.includes('cpu') || n.includes('cluster');
    })
    .sort((a, b) => b.temp_celsius - a.temp_celsius);
}

export function CoreBars({
  cores,
  overallUsage,
  loadAvg,
  thermal = [],
  className,
}: CoreBarsProps) {
  const stats = calcStats(cores);
  const activeCores = cores.filter((c) => c.usage > 5).length;
  const cluster = clusterInfo(cores);
  const clusters = splitClusters(cores);
  const ranked = [...cores].sort((a, b) => b.usage - a.usage);
  const cpuTemps = cpuThermalZones(thermal);
  const coreCount = cores.length || 1;
  const load1Pct = Math.min((loadAvg[0] / coreCount) * 100, 100);

  const freqGroups = new Map<number, number[]>();
  for (const c of cores) {
    const arr = freqGroups.get(c.frequency_mhz) || [];
    arr.push(c.id);
    freqGroups.set(c.frequency_mhz, arr);
  }

  return (
    <Card className={`p-4 sm:p-5 flex flex-col gap-3 h-full min-h-0 ${className ?? ''}`}>
      <div className="flex items-center justify-between shrink-0">
        <span className="text-[10px] font-mono uppercase tracking-widest opacity-50">处理器核心</span>
        <div className="flex items-center gap-2 font-mono text-[10px] opacity-30">
          {cluster && <span>{cluster}</span>}
          <span>{activeCores}/{cores.length} 活跃</span>
        </div>
      </div>

      <div className="grid grid-cols-2 gap-3 shrink-0">
        <div className="rounded-lg border border-default-200 p-2.5">
          <div className="text-[9px] font-mono opacity-40 mb-1">总体 CPU</div>
          <div
            className="font-mono text-2xl font-light leading-none"
            style={{ color: `var(--${percentColor(overallUsage)})` }}
          >
            {overallUsage.toFixed(0)}%
          </div>
        </div>
        <div className="rounded-lg border border-default-200 p-2.5 flex flex-col gap-1">
          <div className="text-[9px] font-mono opacity-40">系统负载</div>
          <div className="font-mono text-[10px] opacity-60">
            {loadAvg.map((v) => v.toFixed(2)).join(' / ')}
          </div>
          <ProgressBar value={load1Pct} size="sm" color={percentColor(load1Pct) as any}>
            <ProgressBar.Track>
              <ProgressBar.Fill />
            </ProgressBar.Track>
          </ProgressBar>
          <div className="text-[9px] font-mono opacity-30">{coreCount} 逻辑核</div>
        </div>
      </div>

      <div className="grid grid-cols-4 gap-1.5 shrink-0">
        {cores.map((core) => (
          <div
            key={core.id}
            className="flex flex-col items-center gap-0.5 p-1.5 rounded-md border border-default-100"
            style={{ background: `${usageBgColor(core.usage)}15` }}
          >
            <span className="font-mono text-[9px] opacity-30">C{core.id}</span>
            <span
              className="font-mono text-sm font-medium leading-none"
              style={{ color: `var(--${percentColor(core.usage)})` }}
            >
              {core.usage.toFixed(0)}%
            </span>
            <span className="font-mono text-[8px] opacity-25">{core.frequency_mhz}</span>
          </div>
        ))}
      </div>

      <div className="flex-1 min-h-0 overflow-y-auto flex flex-col gap-3 pr-1">
        <section>
          <div className="text-[9px] font-mono uppercase tracking-widest opacity-40 mb-1.5">集群概览</div>
          <div className="flex flex-col gap-2">
            {clusters.map((group) => {
              const groupStats = clusterStats(group.cores);
              return (
                <div key={group.label} className="rounded-md border border-default-100 p-2">
                  <div className="flex items-center justify-between gap-2 mb-1">
                    <span className="text-[10px] font-mono opacity-60">{group.label}</span>
                    <span className="text-[9px] font-mono opacity-35">
                      {groupStats.active}/{group.cores.length} 活跃
                    </span>
                  </div>
                  <ProgressBar
                    value={groupStats.avgUsage}
                    size="sm"
                    color={percentColor(groupStats.avgUsage) as any}
                  >
                    <ProgressBar.Track>
                      <ProgressBar.Fill />
                    </ProgressBar.Track>
                  </ProgressBar>
                  <div className="mt-1 flex justify-between text-[9px] font-mono opacity-40">
                    <span>平均 {groupStats.avgUsage.toFixed(1)}%</span>
                    <span>{Math.round(groupStats.avgFreq)} MHz</span>
                  </div>
                </div>
              );
            })}
          </div>
        </section>

        <section>
          <div className="text-[9px] font-mono uppercase tracking-widest opacity-40 mb-1.5">核心排行</div>
          <div className="flex flex-col gap-1.5">
            {ranked.map((core, index) => (
              <div key={core.id} className="flex items-center gap-2 text-xs py-0.5">
                <span className="w-4 font-mono text-[9px] opacity-25">{index + 1}</span>
                <span className="w-5 font-mono text-[10px] opacity-50">C{core.id}</span>
                <div className="flex-1 min-w-0">
                  <ProgressBar value={core.usage} size="sm" color={percentColor(core.usage) as any}>
                    <ProgressBar.Track>
                      <ProgressBar.Fill />
                    </ProgressBar.Track>
                  </ProgressBar>
                </div>
                <span className="w-8 font-mono text-[10px] text-right opacity-60">
                  {core.usage.toFixed(0)}%
                </span>
                <span className="w-10 font-mono text-[9px] text-right opacity-30 hidden sm:inline">
                  {core.frequency_mhz}
                </span>
                <span className="w-8 font-mono text-[9px] text-right opacity-35">
                  {coreStateLabel(core.usage)}
                </span>
              </div>
            ))}
          </div>
        </section>

        {cpuTemps.length > 0 && (
          <section>
            <div className="text-[9px] font-mono uppercase tracking-widest opacity-40 mb-1.5">CPU 温度</div>
            <div className="flex flex-col gap-1.5">
              {cpuTemps.map((zone) => {
                const label = thermalSensorLabel(zone.name);
                return (
                  <div key={zone.id} className="flex items-center gap-2 text-xs py-0.5">
                    <span className="flex-1 min-w-0 text-[10px] truncate opacity-50">{label.title}</span>
                    <div className="w-16 sm:w-20">
                      <ProgressBar
                        value={zone.temp_celsius}
                        maxValue={85}
                        size="sm"
                        color={tempColor(zone.temp_celsius) as any}
                      >
                        <ProgressBar.Track>
                          <ProgressBar.Fill />
                        </ProgressBar.Track>
                      </ProgressBar>
                    </div>
                    <span
                      className="w-10 font-mono text-[10px] text-right"
                      style={{ color: `var(--${tempColor(zone.temp_celsius)})` }}
                    >
                      {zone.temp_celsius.toFixed(1)}°
                    </span>
                  </div>
                );
              })}
            </div>
          </section>
        )}

        {freqGroups.size > 1 && (
          <section>
            <div className="text-[9px] font-mono uppercase tracking-widest opacity-40 mb-1.5">频率分布</div>
            <div className="flex flex-col gap-1 font-mono text-[10px]">
              {[...freqGroups.entries()]
                .sort((a, b) => b[0] - a[0])
                .map(([freq, ids]) => (
                  <div key={freq} className="flex items-center justify-between opacity-50">
                    <span>{freq} MHz</span>
                    <span className="opacity-40">
                      ×{ids.length} · C{ids.join(', C')}
                    </span>
                  </div>
                ))}
            </div>
          </section>
        )}

        {stats && (
          <section className="pt-2 border-t border-default-200 font-mono text-[10px] sm:text-[11px]">
            <div className="text-[9px] uppercase tracking-widest opacity-40 mb-1.5">统计摘要</div>
            <div className="flex flex-col gap-1 opacity-50">
              <span>
                平均 {stats.avg.toFixed(1)}% · 最高 C{stats.maxCoreId} {stats.max.toFixed(0)}% · 最低 C
                {stats.minCoreId} {stats.min.toFixed(0)}%
              </span>
              <span>
                总负载 {stats.totalLoad.toFixed(0)}% · 繁忙 {stats.busy} 核 · 空闲 {stats.idle} 核
              </span>
              <span>
                频率 {stats.freqMin === stats.freqMax ? `${stats.freqMin} MHz` : `${stats.freqMin}–${stats.freqMax} MHz`}
                {' · '}
                均衡度{' '}
                <span className={balanceLabel(stats.stddev).color}>{balanceLabel(stats.stddev).text}</span>
                <span className="opacity-40"> (σ={stats.stddev.toFixed(1)})</span>
              </span>
            </div>
          </section>
        )}
      </div>
    </Card>
  );
}

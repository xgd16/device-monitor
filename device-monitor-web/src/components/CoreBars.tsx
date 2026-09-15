import type { ReactNode } from 'react';
import { MeterRow, Panel, StatCell } from './Panel';
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

function Section({ title, children }: { title: string; children: ReactNode }) {
  return (
    <section className="flex flex-col gap-1.5">
      <div className="dm-sub">{title}</div>
      {children}
    </section>
  );
}

export function CoreBars({ cores, overallUsage, loadAvg, thermal = [], className }: CoreBarsProps) {
  const stats = calcStats(cores);
  const activeCores = cores.filter((c) => c.usage > 5).length;
  const cluster = clusterInfo(cores);
  const clusters = splitClusters(cores);
  const ranked = [...cores].sort((a, b) => b.usage - a.usage);
  const cpuTemps = cpuThermalZones(thermal);
  const maxFreq = Math.max(...cores.map((c) => c.frequency_mhz), 0);

  const freqGroups = new Map<number, number[]>();
  for (const c of cores) {
    const arr = freqGroups.get(c.frequency_mhz) || [];
    arr.push(c.id);
    freqGroups.set(c.frequency_mhz, arr);
  }

  return (
    <Panel
      label="处理器核心"
      index={6}
      className={`h-full ${className ?? ''}`}
      hint={
        <>
          {cluster && <span>{cluster}</span>}
          <span>
            {activeCores}/{cores.length} 活跃
          </span>
        </>
      }
      bodyClassName="gap-3"
    >
      <div className="grid shrink-0 grid-cols-2 gap-2.5">
        <StatCell
          label="总体 CPU"
          value={overallUsage.toFixed(0)}
          unit="%"
          color={percentColor(overallUsage)}
          sub={`${activeCores} 核活跃`}
        />
        <StatCell
          label="峰值频率"
          value={maxFreq}
          unit="MHz"
          sub={`负载 ${loadAvg.map((v) => v.toFixed(2)).join(' / ')}`}
        />
      </div>

      {/* 每核占用：8 核即 4×2，天然对称 */}
      <div className="grid shrink-0 grid-cols-4 gap-1.5">
        {cores.map((core) => (
          <div
            key={core.id}
            className="flex flex-col items-center gap-0.5 rounded-md border border-default-100 px-1 py-1.5"
          >
            <span className="font-mono text-[9px] opacity-35">C{core.id}</span>
            <span
              className="dm-hero text-sm leading-none"
              style={{ color: `var(--${percentColor(core.usage)})` }}
            >
              {core.usage.toFixed(0)}
            </span>
            <span className="font-mono text-[8px] opacity-30">{core.frequency_mhz}</span>
          </div>
        ))}
      </div>

      <div className="flex min-h-0 flex-1 flex-col gap-3 overflow-y-auto pr-1">
        <Section title="集群概览">
          <div className="flex flex-col gap-2">
            {clusters.map((group) => {
              const groupStats = clusterStats(group.cores);
              return (
                <div key={group.label} className="dm-inset flex flex-col gap-1.5 p-2">
                  <div className="flex items-center justify-between gap-2">
                    <span className="font-mono text-[10px] opacity-60">{group.label}</span>
                    <span className="font-mono text-[9px] opacity-35">
                      {groupStats.active}/{group.cores.length} 活跃
                    </span>
                  </div>
                  <MeterRow
                    label="使用率"
                    ratio={groupStats.avgUsage}
                    color={percentColor(groupStats.avgUsage)}
                    value={`${groupStats.avgUsage.toFixed(1)}%`}
                  />
                  <MeterRow
                    label="平均频率"
                    ratio={maxFreq > 0 ? (groupStats.avgFreq / maxFreq) * 100 : 0}
                    value={`${Math.round(groupStats.avgFreq)}M`}
                  />
                </div>
              );
            })}
          </div>
        </Section>

        <Section title="核心排行">
          <div className="flex flex-col gap-1.5">
            {ranked.map((core, index) => (
              <div key={core.id} className="flex items-center gap-2">
                <span className="w-3 shrink-0 font-mono text-[9px] opacity-25">{index + 1}</span>
                <span className="w-5 shrink-0 font-mono text-[10px] opacity-50">C{core.id}</span>
                <div className="min-w-0 flex-1">
                  <span className="block h-1 overflow-hidden rounded-full bg-default-200">
                    <span
                      className="block h-full rounded-full transition-[width] duration-500"
                      style={{
                        width: `${core.usage}%`,
                        background: `var(--${percentColor(core.usage)})`,
                      }}
                    />
                  </span>
                </div>
                <span className="hidden w-10 shrink-0 text-right font-mono text-[9px] opacity-30 sm:inline">
                  {core.frequency_mhz}
                </span>
                <span className="w-8 shrink-0 text-right font-mono text-[9px] opacity-35">
                  {coreStateLabel(core.usage)}
                </span>
              </div>
            ))}
          </div>
        </Section>

        {cpuTemps.length > 0 && (
          <Section title="CPU 温度">
            <div className="flex flex-col gap-1.5">
              {cpuTemps.map((zone) => (
                <MeterRow
                  key={zone.id}
                  label={thermalSensorLabel(zone.name).title}
                  title={zone.name}
                  ratio={(zone.temp_celsius / 85) * 100}
                  color={tempColor(zone.temp_celsius)}
                  value={`${zone.temp_celsius.toFixed(1)}°`}
                />
              ))}
            </div>
          </Section>
        )}

        {freqGroups.size > 1 && (
          <Section title="频率分布">
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
          </Section>
        )}

        {stats && (
          <Section title="统计摘要">
            <div className="flex flex-col gap-1 font-mono text-[10px] opacity-50 xl:text-[11px]">
              <span>
                平均 {stats.avg.toFixed(1)}% · 最高 C{stats.maxCoreId} {stats.max.toFixed(0)}% ·
                最低 C{stats.minCoreId} {stats.min.toFixed(0)}%
              </span>
              <span>
                总负载 {stats.totalLoad.toFixed(0)}% · 繁忙 {stats.busy} 核 · 空闲 {stats.idle} 核
              </span>
              <span>
                频率{' '}
                {stats.freqMin === stats.freqMax
                  ? `${stats.freqMin} MHz`
                  : `${stats.freqMin}–${stats.freqMax} MHz`}
                {' · '}均衡度{' '}
                <span className={balanceLabel(stats.stddev).color}>
                  {balanceLabel(stats.stddev).text}
                </span>
                <span className="opacity-40"> (σ={stats.stddev.toFixed(1)})</span>
              </span>
            </div>
          </Section>
        )}
      </div>
    </Panel>
  );
}

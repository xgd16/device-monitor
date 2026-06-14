import { Card, ProgressBar } from '@heroui/react';
import { percentColor } from './utils';
import type { CpuCore } from '../types';

interface CoreBarsProps {
  cores: CpuCore[];
}

function calcStats(cores: CpuCore[]) {
  if (cores.length === 0) return null;
  const usages = cores.map(c => c.usage);
  const freqs = cores.map(c => c.frequency_mhz);
  const avg = usages.reduce((a, b) => a + b, 0) / cores.length;
  const max = Math.max(...usages);
  const min = Math.min(...usages);
  const maxCore = cores.find(c => c.usage === max);
  const minCore = cores.find(c => c.usage === min);
  const variance = usages.reduce((s, u) => s + (u - avg) ** 2, 0) / cores.length;
  return {
    avg, max, min,
    maxCoreId: maxCore?.id ?? 0,
    minCoreId: minCore?.id ?? 0,
    freqMin: Math.min(...freqs),
    freqMax: Math.max(...freqs),
    stddev: Math.sqrt(variance),
    busy: cores.filter(c => c.usage > 30).length,
    idle: cores.filter(c => c.usage < 5).length,
    totalLoad: usages.reduce((a, b) => a + b, 0),
  };
}

function balanceLabel(stddev: number): { text: string; color: string } {
  if (stddev < 10) return { text: '均衡', color: 'text-success' };
  if (stddev < 25) return { text: '轻度倾斜', color: 'text-warning' };
  return { text: '严重倾斜', color: 'text-danger' };
}

// 颜色映射：usage -> 背景色
function usageBgColor(usage: number): string {
  if (usage > 80) return 'var(--danger)';
  if (usage > 50) return 'var(--warning)';
  if (usage > 20) return 'var(--accent)';
  if (usage > 5) return 'var(--default)';
  return 'transparent';
}

function clusterInfo(cores: CpuCore[]): string {
  if (cores.length <= 1) return '';
  // 按核心数量推断集群（SDM845: 4+4）
  const half = Math.floor(cores.length / 2);
  // 用 ID 范围区分：小核 0-3，大核 4-7
  const smallMaxFreq = Math.max(...cores.filter(c => c.id < half).map(c => c.frequency_mhz));
  const bigMaxFreq = Math.max(...cores.filter(c => c.id >= half).map(c => c.frequency_mhz));
  if (smallMaxFreq !== bigMaxFreq) {
    return `${half}大@${bigMaxFreq} + ${cores.length - half}小@${smallMaxFreq}`;
  }
  return `${cores.length} 核`;
}

export function CoreBars({ cores }: CoreBarsProps) {
  const stats = calcStats(cores);
  const activeCores = cores.filter(c => c.usage > 5).length;
  const cluster = clusterInfo(cores);

  // 频率分组
  const freqGroups = new Map<number, number[]>();
  for (const c of cores) {
    const arr = freqGroups.get(c.frequency_mhz) || [];
    arr.push(c.id);
    freqGroups.set(c.frequency_mhz, arr);
  }

  return (
    <Card className="p-4 sm:p-5 flex flex-col gap-3">
      {/* 标题行 */}
      <div className="flex items-center justify-between">
        <span className="text-[10px] font-mono uppercase tracking-widest opacity-50">处理器核心</span>
        <div className="flex items-center gap-2 font-mono text-[10px] opacity-30">
          {cluster && <span>{cluster}</span>}
          <span>{activeCores}/{cores.length} 活跃</span>
        </div>
      </div>

      {/* 核心热力图网格 */}
      <div className="grid grid-cols-4 gap-1.5">
        {cores.map(core => (
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

      {/* 进度条列表 */}
      <div className="flex flex-col gap-1.5">
        {cores.map(core => (
          <div key={core.id} className="flex items-center gap-2 text-xs">
            <span className="w-5 font-mono text-[10px] text-right opacity-40">{core.id}</span>
            <div className="flex-1">
              <ProgressBar value={core.usage} size="sm" color={percentColor(core.usage) as any}>
                <ProgressBar.Track>
                  <ProgressBar.Fill />
                </ProgressBar.Track>
              </ProgressBar>
            </div>
            <span className="w-9 font-mono text-[10px] text-right opacity-60">{core.usage.toFixed(0)}%</span>
            <span className="w-12 font-mono text-[9px] text-right opacity-30">{core.frequency_mhz}</span>
          </div>
        ))}
      </div>

      {/* 频率分布 */}
      {freqGroups.size > 1 && (
        <div className="flex flex-wrap gap-2">
          {[...freqGroups.entries()].sort((a, b) => b[0] - a[0]).map(([freq, ids]) => (
            <div key={freq} className="flex items-center gap-1 font-mono text-[10px]">
              <span className="inline-block w-2 h-2 rounded-full bg-accent opacity-40" />
              <span className="opacity-50">{freq} MHz</span>
              <span className="opacity-25">×{ids.length}</span>
            </div>
          ))}
        </div>
      )}

      {/* 统计分析 */}
      {stats && (
        <div className="pt-2.5 border-t border-default-200 flex flex-col gap-1.5 font-mono text-[10px] sm:text-[11px]">
          <div className="flex flex-wrap gap-x-4 gap-y-1">
            <span className="opacity-50">平均 <span className="opacity-80">{stats.avg.toFixed(1)}%</span></span>
            <span className="text-danger opacity-70">最高 core{stats.maxCoreId} <span className="opacity-100">{stats.max.toFixed(0)}%</span></span>
            <span className="text-success opacity-70">最低 core{stats.minCoreId} <span className="opacity-100">{stats.min.toFixed(0)}%</span></span>
          </div>
          <div className="flex flex-wrap gap-x-4 gap-y-1">
            <span className="opacity-50">总负载 {stats.totalLoad.toFixed(0)}%</span>
            <span className="opacity-50">繁忙 <span className="text-warning">{stats.busy}</span> 核</span>
            <span className="opacity-50">空闲 <span className="text-success">{stats.idle}</span> 核</span>
          </div>
          <div className="flex flex-wrap gap-x-4 gap-y-1">
            <span className="opacity-50">
              频率 {stats.freqMin === stats.freqMax ? `${stats.freqMin} MHz` : `${stats.freqMin}–${stats.freqMax} MHz`}
            </span>
            <span className="opacity-50">
              均衡度 <span className={balanceLabel(stats.stddev).color}>{balanceLabel(stats.stddev).text}</span>
              <span className="opacity-40"> (σ={stats.stddev.toFixed(1)})</span>
            </span>
          </div>
        </div>
      )}
    </Card>
  );
}

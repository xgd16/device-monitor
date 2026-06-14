import { Card, ProgressBar } from '@heroui/react';
import type { SystemOverview, ProcessInfo } from '../types';
import { tempColor, percentColor } from './utils';

interface MetricsBarProps {
  data: SystemOverview;
  processes: ProcessInfo[];
}

export function MetricsBar({ data, processes }: MetricsBarProps) {
  const maxTemp = Math.max(...data.thermal.map(t => t.temp_celsius), 0);
  const hotZones = data.thermal.filter(t => t.temp_celsius >= 60).length;
  const top3Thermal = [...data.thermal].sort((a, b) => b.temp_celsius - a.temp_celsius).slice(0, 3);

  const load1 = data.load_avg[0];
  const load5 = data.load_avg[1];
  const load15 = data.load_avg[2];
  const cores = data.cpu.cores.length || 1;
  const loadPct = Math.min((load1 / cores) * 100, 100);
  const loadTrend = load1 > load15 * 1.15 ? '↑' : load1 < load15 * 0.85 ? '↓' : '→';

  const runningProcs = processes.filter(p => p.status.includes('run')).length;
  const totalMem = processes.reduce((s, p) => s + p.memory_mb, 0);
  const top5Cpu = [...processes].sort((a, b) => b.cpu_usage - a.cpu_usage).slice(0, 5);
  const maxCpuProc = top5Cpu[0]?.cpu_usage || 0;

  return (
    <>
      {/* 温度 */}
      <Card className="p-3 sm:p-4 flex flex-col gap-2">
        <span className="text-[9px] sm:text-[10px] font-mono uppercase tracking-widest opacity-50">温度</span>
        <div className="flex items-baseline gap-2">
          <span
            className="font-mono text-xl sm:text-2xl lg:text-3xl font-light leading-none"
            style={{ color: `var(--${tempColor(maxTemp)})` }}
          >
            {maxTemp.toFixed(1)}<span className="text-[10px] opacity-50">°C</span>
          </span>
          {hotZones > 0 && (
            <span className="text-[9px] sm:text-[10px] font-mono text-warning opacity-70">{hotZones} 个高温区</span>
          )}
        </div>
        <div className="flex flex-col gap-0.5">
          {top3Thermal.map(z => (
            <div key={z.id} className="flex items-center gap-1.5 text-[9px] sm:text-[10px] font-mono">
              <span className="flex-1 truncate opacity-40">{z.name.replace('-thermal', '')}</span>
              <span style={{ color: `var(--${tempColor(z.temp_celsius)})` }}>{z.temp_celsius.toFixed(1)}°</span>
            </div>
          ))}
        </div>
        <span className="text-[9px] font-mono opacity-25">{data.thermal.length} 个传感器</span>
      </Card>

      {/* 负载 */}
      <Card className="p-3 sm:p-4 flex flex-col gap-2">
        <span className="text-[9px] sm:text-[10px] font-mono uppercase tracking-widest opacity-50">负载</span>
        <div className="flex items-baseline gap-2">
          <span className="font-mono text-xl sm:text-2xl lg:text-3xl font-light leading-none">
            {load1.toFixed(2)}
          </span>
          <span className="text-[10px] font-mono opacity-30">{loadTrend} {cores} 核</span>
        </div>
        <ProgressBar value={loadPct} size="sm" color={percentColor(loadPct) as any}>
          <ProgressBar.Track>
            <ProgressBar.Fill />
          </ProgressBar.Track>
        </ProgressBar>
        <div className="flex gap-3 text-[9px] sm:text-[10px] font-mono opacity-40">
          <span>5分 {load5.toFixed(2)}</span>
          <span>15分 {load15.toFixed(2)}</span>
        </div>
      </Card>

      {/* 进程 */}
      <Card className="p-3 sm:p-4 flex flex-col gap-2">
        <span className="text-[9px] sm:text-[10px] font-mono uppercase tracking-widest opacity-50">进程</span>
        <div className="flex items-baseline gap-2">
          <span className="font-mono text-xl sm:text-2xl lg:text-3xl font-light leading-none">
            {data.process_count}
          </span>
          <span className="text-[9px] sm:text-[10px] font-mono text-success opacity-60">{runningProcs} 运行</span>
        </div>
        <div className="flex flex-col gap-0.5">
          {top5Cpu.filter(p => p.cpu_usage > 0.5).map(p => (
            <div key={p.pid} className="flex items-center gap-1.5 text-[9px] sm:text-[10px] font-mono">
              <span className="flex-1 truncate opacity-40">{p.name}</span>
              <span
                style={{ color: p.cpu_usage > 20 ? 'var(--warning)' : p.cpu_usage > 50 ? 'var(--danger)' : undefined }}
              >
                {p.cpu_usage.toFixed(1)}%
              </span>
            </div>
          ))}
          {maxCpuProc <= 0.5 && (
            <span className="text-[9px] font-mono opacity-25">CPU 空闲</span>
          )}
        </div>
        <span className="text-[9px] font-mono opacity-25">内存总计 {totalMem >= 1024 ? `${(totalMem / 1024).toFixed(1)}G` : `${totalMem}M`}</span>
      </Card>
    </>
  );
}

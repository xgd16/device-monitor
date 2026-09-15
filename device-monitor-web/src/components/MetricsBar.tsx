import type { ReactNode } from 'react';
import { Hero, MeterRow, Panel } from './Panel';
import type { SystemOverview, ProcessInfo } from '../types';
import { tempColor, percentColor, thermalSensorLabel, fmtMem } from './utils';

interface MetricsBarProps {
  data: SystemOverview;
  processes: ProcessInfo[];
}

/** 面板页脚：一行低对比度说明文字 */
function Foot({ children }: { children: ReactNode }) {
  return (
    <div className="mt-auto flex flex-wrap items-center justify-center gap-x-2 border-t border-default-100 pt-2 font-mono text-[9px] opacity-40 xl:text-[10px]">
      {children}
    </div>
  );
}

export function MetricsBar({ data, processes }: MetricsBarProps) {
  const thermal = [...data.thermal].sort((a, b) => b.temp_celsius - a.temp_celsius);
  const maxTemp = thermal.length > 0 ? thermal[0].temp_celsius : 0;
  const hotZones = data.thermal.filter((t) => t.temp_celsius >= 60).length;

  const [load1, load5, load15] = data.load_avg;
  const cores = data.cpu.cores.length || 1;
  const loadPct = Math.min((load1 / cores) * 100, 100);
  const loadTrend = load1 > load15 * 1.15 ? '↑' : load1 < load15 * 0.85 ? '↓' : '→';

  const runningProcs = processes.filter((p) => p.status.includes('run')).length;
  const totalMem = processes.reduce((s, p) => s + p.memory_mb, 0);
  const topCpu = [...processes]
    .sort((a, b) => b.cpu_usage - a.cpu_usage)
    .filter((p) => p.cpu_usage > 0.5)
    .slice(0, 3);

  return (
    <>
      {/* 温度 */}
      <Panel label="温度" index={2} hint={`${data.thermal.length} 传感器`}>
        <Hero
          value={maxTemp.toFixed(1)}
          unit="°C"
          color={tempColor(maxTemp)}
          note={hotZones > 0 ? `${hotZones} 个高温区` : '全区域正常'}
        />
        <div className="flex flex-col gap-1.5">
          {thermal.slice(0, 3).map((z) => (
            <MeterRow
              key={z.id}
              label={thermalSensorLabel(z.name).title}
              title={z.name}
              ratio={(z.temp_celsius / 85) * 100}
              color={tempColor(z.temp_celsius)}
              value={`${z.temp_celsius.toFixed(1)}°`}
            />
          ))}
        </div>
        <Foot>
          <span>报警阈值 60°C</span>
          <span>·</span>
          <span>峰值 {thermal[0]?.name.replace(/-thermal$/i, '') ?? '—'}</span>
        </Foot>
      </Panel>

      {/* 负载 */}
      <Panel label="负载" index={3} hint={`${cores} 逻辑核`}>
        <Hero value={load1.toFixed(2)} note={`${loadTrend} 1 分钟`} />
        <div className="flex flex-col gap-1.5">
          {[
            { label: '1 分钟', v: load1 },
            { label: '5 分钟', v: load5 },
            { label: '15 分钟', v: load15 },
          ].map((r) => {
            const pct = Math.min((r.v / cores) * 100, 100);
            return (
              <MeterRow
                key={r.label}
                label={r.label}
                ratio={pct}
                color={percentColor(pct)}
                value={r.v.toFixed(2)}
              />
            );
          })}
        </div>
        <Foot>
          <span>相对 {cores} 核</span>
          <span>·</span>
          <span>当前 {loadPct.toFixed(0)}%</span>
        </Foot>
      </Panel>

      {/* 进程 */}
      <Panel label="进程" index={4} hint={`${runningProcs} 运行`}>
        <Hero value={data.process_count} note="总进程数" />
        <div className="flex flex-col gap-1.5">
          {topCpu.length === 0 && (
            <span className="font-mono text-[10px] opacity-30">无显著 CPU 占用</span>
          )}
          {topCpu.map((p) => (
            <MeterRow
              key={p.pid}
              label={p.name}
              title={`PID ${p.pid}`}
              ratio={p.cpu_usage}
              color={p.cpu_usage > 50 ? 'danger' : p.cpu_usage > 20 ? 'warning' : undefined}
              value={`${p.cpu_usage.toFixed(1)}%`}
            />
          ))}
        </div>
        <Foot>
          <span>合计占用 {fmtMem(totalMem)}</span>
          <span>·</span>
          <span>列出占用最高的 3 个</span>
        </Foot>
      </Panel>
    </>
  );
}

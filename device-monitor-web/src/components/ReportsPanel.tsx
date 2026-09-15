import { useCallback, useEffect, useState } from 'react';
import { Spinner, Button } from '@heroui/react';
import { fetchHistoryMetrics, fetchDatabaseStats } from '../api';
import type { HistorySeries, DatabaseStats } from '../types';
import { HistoryChart } from './HistoryChart';
import { chartPalette, useChartTheme } from './chartTheme';
import { Panel } from './Panel';

const RANGES = [
  { key: '1h', label: '1 小时' },
  { key: '6h', label: '6 小时' },
  { key: '24h', label: '24 小时' },
  { key: '7d', label: '7 天' },
] as const;

type RangeKey = (typeof RANGES)[number]['key'];

function fmtRangeTime(ts: number) {
  return new Date(ts * 1000).toLocaleString('zh-CN', { hour12: false });
}

export function ReportsPanel() {
  const [range, setRange] = useState<RangeKey>('24h');
  const [series, setSeries] = useState<HistorySeries | null>(null);
  const [stats, setStats] = useState<DatabaseStats | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState('');
  const theme = useChartTheme();
  const pal = chartPalette(theme);

  const load = useCallback(async () => {
    setLoading(true);
    setError('');
    try {
      const [hist, dbStats] = await Promise.all([
        fetchHistoryMetrics(range),
        fetchDatabaseStats().catch(() => null),
      ]);
      setSeries(hist);
      setStats(dbStats);
    } catch {
      setError('加载历史数据失败');
      setSeries(null);
    } finally {
      setLoading(false);
    }
  }, [range]);

  useEffect(() => {
    load();
  }, [load]);

  const ts = series?.timestamps ?? [];

  return (
    <div className="flex flex-col gap-3">
      {/* 工具栏 */}
      <Panel
        label="历史报表"
        index={23}
        hint={
          <>
            {RANGES.map((r) => (
              <Button
                key={r.key}
                size="sm"
                variant={range === r.key ? 'secondary' : 'ghost'}
                onPress={() => setRange(r.key)}
                className="font-mono text-xs"
              >
                {r.label}
              </Button>
            ))}
            <Button size="sm" variant="ghost" onPress={load} className="font-mono text-xs">
              刷新
            </Button>
          </>
        }
        bodyClassName="gap-2"
      >
        {series && series.count > 0 ? (
          <div className="flex flex-wrap items-center justify-center gap-x-2 font-mono text-[10px] opacity-45 xl:text-[11px]">
            <span>
              {fmtRangeTime(series.from)} — {fmtRangeTime(series.to)}
            </span>
            <span>·</span>
            <span>{series.count} 个采样点</span>
            {stats && (
              <>
                <span>·</span>
                <span>库内共 {stats.metrics_count} 条</span>
              </>
            )}
          </div>
        ) : (
          <span className="font-mono text-[10px] opacity-35">暂无采样数据</span>
        )}
      </Panel>

      {loading && (
        <div className="flex items-center justify-center py-16">
          <Spinner size="lg" />
        </div>
      )}

      {!loading && error && (
        <Panel label="历史报表" index={23}>
          <p className="py-6 text-center text-sm opacity-50">{error}</p>
        </Panel>
      )}

      {!loading && !error && series && series.count < 2 && (
        <Panel label="历史报表" index={23} bodyClassName="gap-2">
          <p className="py-4 text-center text-sm opacity-60">该时间范围内暂无足够历史数据</p>
          <p className="text-center font-mono text-[11px] opacity-40">
            服务每 5 秒采集一次，数据保留 7 天。请稍后再试或缩短时间范围。
          </p>
        </Panel>
      )}

      {!loading && !error && series && series.count >= 2 && (
        <div className="grid grid-cols-1 lg:grid-cols-2 gap-3">
          <Panel label="CPU 使用率" index={24} bodyClassName="gap-2">
            <HistoryChart
              timestamps={ts}
              range={range}
              series={[
                { name: 'CPU %', data: series.cpu_usage, color: pal.accent, unit: '%', area: true },
              ]}
            />
          </Panel>

          <Panel label="内存" index={24} bodyClassName="gap-2">
            <HistoryChart
              timestamps={ts}
              range={range}
              series={[
                { name: '使用率 %', data: series.memory_percent, color: pal.success, area: true },
                { name: '已用 MB', data: series.memory_used_mb, color: pal.success, yAxisIndex: 1 },
              ]}
            />
          </Panel>

          <Panel label="系统负载" index={24} bodyClassName="gap-2">
            <HistoryChart
              timestamps={ts}
              range={range}
              series={[
                { name: '1 min', data: series.load_1, color: pal.accent },
                { name: '5 min', data: series.load_5, color: pal.violet },
                { name: '15 min', data: series.load_15, color: pal.violet },
              ]}
            />
          </Panel>

          <Panel label="电池" index={24} bodyClassName="gap-2">
            <HistoryChart
              timestamps={ts}
              range={range}
              series={[
                { name: '电量 %', data: series.battery_capacity, color: pal.warning, area: true },
                { name: '功率 W', data: series.battery_power_w, color: pal.warning, yAxisIndex: 1 },
              ]}
            />
          </Panel>

          <Panel label="最高温度" index={24} bodyClassName="gap-2">
            <HistoryChart
              timestamps={ts}
              range={range}
              series={[{ name: '°C', data: series.thermal_max, color: pal.danger, area: true }]}
            />
          </Panel>

          <Panel label="网络流量" index={24} bodyClassName="gap-2">
            <HistoryChart
              timestamps={ts}
              range={range}
              series={[
                { name: '下载 KB/s', data: series.network_rx_kbps, color: pal.accent, area: true },
                { name: '上传 KB/s', data: series.network_tx_kbps, color: pal.warning, area: true },
              ]}
            />
          </Panel>

          <Panel label="进程数" index={24} className="lg:col-span-2" bodyClassName="gap-2">
            <HistoryChart
              timestamps={ts}
              range={range}
              height={180}
              series={[
                { name: '进程数', data: series.process_count, color: pal.neutral, area: true },
              ]}
            />
          </Panel>
        </div>
      )}
    </div>
  );
}

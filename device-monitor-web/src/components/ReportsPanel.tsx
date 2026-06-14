import { useCallback, useEffect, useState } from 'react';
import { Card, Spinner, Button } from '@heroui/react';
import { fetchHistoryMetrics, fetchDatabaseStats } from '../api';
import type { HistorySeries, DatabaseStats } from '../types';
import { HistoryChart } from './HistoryChart';

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

  useEffect(() => { load(); }, [load]);

  const ts = series?.timestamps ?? [];

  return (
    <div className="flex flex-col gap-3">
      {/* 工具栏 */}
      <Card className="p-3 sm:p-4 flex flex-col sm:flex-row sm:items-center justify-between gap-3">
        <div>
          <span className="text-[10px] font-mono uppercase tracking-widest opacity-50">历史报表</span>
          {series && series.count > 0 && (
            <p className="text-[10px] sm:text-[11px] font-mono opacity-40 mt-1">
              {fmtRangeTime(series.from)} — {fmtRangeTime(series.to)} · {series.count} 个采样点
              {stats && ` · 库内共 ${stats.metrics_count} 条`}
            </p>
          )}
        </div>
        <div className="flex flex-wrap items-center gap-1.5">
          {RANGES.map((r) => (
            <Button
              key={r.key}
              size="sm"
              variant={range === r.key ? 'secondary' : 'ghost'}
              onPress={() => setRange(r.key)}
            >
              {r.label}
            </Button>
          ))}
          <Button size="sm" variant="ghost" onPress={load}>刷新</Button>
        </div>
      </Card>

      {loading && (
        <div className="flex items-center justify-center py-16">
          <Spinner size="lg" />
        </div>
      )}

      {!loading && error && (
        <Card className="p-8 text-center text-sm opacity-50">{error}</Card>
      )}

      {!loading && !error && series && series.count < 2 && (
        <Card className="p-8 text-center flex flex-col gap-2">
          <p className="text-sm opacity-60">该时间范围内暂无足够历史数据</p>
          <p className="text-[11px] font-mono opacity-40">
            服务每 5 秒采集一次，数据保留 7 天。请稍后再试或缩短时间范围。
          </p>
        </Card>
      )}

      {!loading && !error && series && series.count >= 2 && (
        <div className="grid grid-cols-1 lg:grid-cols-2 gap-3">
          <Card className="p-3 sm:p-4">
            <HistoryChart
              title="CPU 使用率"
              timestamps={ts}
              range={range}
              series={[
                { name: 'CPU %', data: series.cpu_usage, color: '#38bdf8', unit: '%', area: true },
              ]}
              yAxisNames={['%']}
            />
          </Card>

          <Card className="p-3 sm:p-4">
            <HistoryChart
              title="内存"
              timestamps={ts}
              range={range}
              series={[
                { name: '使用率 %', data: series.memory_percent, color: '#4ade80', area: true },
                { name: '已用 MB', data: series.memory_used_mb, color: '#a3e635', yAxisIndex: 1 },
              ]}
              yAxisNames={['%', 'MB']}
            />
          </Card>

          <Card className="p-3 sm:p-4">
            <HistoryChart
              title="系统负载"
              timestamps={ts}
              range={range}
              series={[
                { name: '1 min', data: series.load_1, color: '#38bdf8' },
                { name: '5 min', data: series.load_5, color: '#818cf8' },
                { name: '15 min', data: series.load_15, color: '#a78bfa' },
              ]}
            />
          </Card>

          <Card className="p-3 sm:p-4">
            <HistoryChart
              title="电池"
              timestamps={ts}
              range={range}
              series={[
                { name: '电量 %', data: series.battery_capacity, color: '#facc15', area: true },
                { name: '功率 W', data: series.battery_power_w, color: '#fb923c', yAxisIndex: 1 },
              ]}
              yAxisNames={['%', 'W']}
            />
          </Card>

          <Card className="p-3 sm:p-4">
            <HistoryChart
              title="最高温度"
              timestamps={ts}
              range={range}
              series={[
                { name: '°C', data: series.thermal_max, color: '#f87171', area: true },
              ]}
              yAxisNames={['°C']}
            />
          </Card>

          <Card className="p-3 sm:p-4">
            <HistoryChart
              title="网络流量"
              timestamps={ts}
              range={range}
              series={[
                { name: '下载 KB/s', data: series.network_rx_kbps, color: '#38bdf8', area: true },
                { name: '上传 KB/s', data: series.network_tx_kbps, color: '#fb923c', area: true },
              ]}
              yAxisNames={['KB/s']}
            />
          </Card>

          <Card className="p-3 sm:p-4 lg:col-span-2">
            <HistoryChart
              title="进程数"
              timestamps={ts}
              range={range}
              height={180}
              series={[
                { name: '进程数', data: series.process_count, color: '#94a3b8', area: true },
              ]}
            />
          </Card>
        </div>
      )}
    </div>
  );
}

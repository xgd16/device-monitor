import { useEffect, useMemo, useState } from 'react';
import ReactECharts from 'echarts-for-react';
import type { EChartsOption } from 'echarts';

interface TrendChartProps {
  data: number[];
  timestamps?: number[];
  variant: 'cpu' | 'mem' | 'gpu';
  unit?: string;
  height?: number;
}

const VARIANT_COLORS = {
  cpu: { line: '#38bdf8', areaTop: 'rgba(56, 189, 248, 0.35)', areaBottom: 'rgba(56, 189, 248, 0.02)' },
  mem: { line: '#4ade80', areaTop: 'rgba(74, 222, 128, 0.35)', areaBottom: 'rgba(74, 222, 128, 0.02)' },
  gpu: { line: '#a78bfa', areaTop: 'rgba(167, 139, 250, 0.35)', areaBottom: 'rgba(167, 139, 250, 0.02)' },
};

function fmtTime(ts: number) {
  return new Date(ts * 1000).toLocaleTimeString('zh-CN', {
    hour12: false,
    hour: '2-digit',
    minute: '2-digit',
    second: '2-digit',
  });
}

export function TrendChart({ data, timestamps, variant, unit = '%', height = 80 }: TrendChartProps) {
  const [theme, setTheme] = useState<'dark' | 'light'>('dark');

  useEffect(() => {
    const el = document.documentElement;
    const read = () => setTheme((el.getAttribute('data-theme') as 'dark' | 'light') || 'dark');
    read();
    const obs = new MutationObserver(read);
    obs.observe(el, { attributes: true, attributeFilter: ['data-theme'] });
    return () => obs.disconnect();
  }, []);

  const option = useMemo<EChartsOption>(() => {
    if (data.length < 2) return {};

    const colors = VARIANT_COLORS[variant];
    const isDark = theme === 'dark';
    const maxVal = Math.max(...data, 1);
    const yMax = variant === 'gpu'
      ? Math.ceil(maxVal * 1.15 + 20)
      : Math.min(100, Math.ceil(maxVal * 1.2 + 8));

    const labels =
      timestamps && timestamps.length === data.length
        ? timestamps.map(fmtTime)
        : data.map((_, i) => `#${i + 1}`);

    return {
      animation: true,
      animationDuration: 300,
      grid: { left: 2, right: 6, top: 10, bottom: 6 },
      tooltip: {
        trigger: 'axis',
        confine: true,
        backgroundColor: isDark ? 'rgba(22, 22, 28, 0.96)' : 'rgba(255, 255, 255, 0.98)',
        borderColor: isDark ? 'rgba(255,255,255,0.12)' : 'rgba(0,0,0,0.08)',
        borderWidth: 1,
        padding: [8, 12],
        textStyle: {
          color: isDark ? '#e4e4e7' : '#27272a',
          fontSize: 11,
          fontFamily: 'ui-monospace, SFMono-Regular, Menlo, monospace',
        },
        formatter: (params: unknown) => {
          const p = (Array.isArray(params) ? params[0] : params) as {
            axisValue?: string;
            value?: number;
          };
          const val = typeof p.value === 'number' ? p.value.toFixed(1) : String(p.value ?? '');
          return `<div style="line-height:1.5">
            <div style="opacity:0.55;font-size:10px">${p.axisValue ?? ''}</div>
            <div><span style="color:${colors.line}">●</span> <b>${val}${unit}</b></div>
          </div>`;
        },
        axisPointer: {
          type: 'cross',
          crossStyle: { color: colors.line, opacity: 0.35 },
          lineStyle: { color: colors.line, opacity: 0.25, type: 'dashed' },
          label: {
            backgroundColor: isDark ? '#3f3f46' : '#e4e4e7',
            color: isDark ? '#fafafa' : '#18181b',
            fontSize: 10,
            fontFamily: 'ui-monospace, monospace',
          },
        },
      },
      xAxis: {
        type: 'category',
        data: labels,
        boundaryGap: false,
        axisLine: { show: false },
        axisTick: { show: false },
        axisLabel: { show: false },
      },
      yAxis: {
        type: 'value',
        min: 0,
        max: yMax,
        splitNumber: 3,
        axisLine: { show: false },
        axisTick: { show: false },
        axisLabel: { show: false },
        splitLine: {
          show: true,
          lineStyle: {
            color: isDark ? 'rgba(255,255,255,0.06)' : 'rgba(0,0,0,0.06)',
            type: 'dashed',
          },
        },
      },
      series: [
        {
          type: 'line',
          data,
          smooth: 0.4,
          symbol: 'circle',
          symbolSize: 6,
          showSymbol: false,
          emphasis: {
            focus: 'series',
            scale: 1.6,
            itemStyle: {
              color: colors.line,
              borderColor: isDark ? '#18181b' : '#fff',
              borderWidth: 2,
              shadowBlur: 8,
              shadowColor: colors.line,
            },
          },
          lineStyle: { width: 2.5, color: colors.line, cap: 'round' },
          areaStyle: {
            color: {
              type: 'linear',
              x: 0,
              y: 0,
              x2: 0,
              y2: 1,
              colorStops: [
                { offset: 0, color: colors.areaTop },
                { offset: 1, color: colors.areaBottom },
              ],
            },
          },
        },
      ],
    };
  }, [data, timestamps, variant, unit, theme]);

  if (data.length < 2) {
    return (
      <div
        style={{ height }}
        className="flex items-center justify-center rounded-lg border border-dashed border-default-200 text-[10px] font-mono opacity-35"
      >
        等待历史数据...
      </div>
    );
  }

  return (
    <ReactECharts
      option={option}
      style={{ height, width: '100%' }}
      opts={{ renderer: 'canvas' }}
      notMerge
    />
  );
}

import { useEffect, useMemo, useState } from 'react';
import ReactECharts from 'echarts-for-react';
import type { EChartsOption } from 'echarts';
import { areaGradient, chartPalette, chartThemeColors } from './chartTheme';

interface TrendChartProps {
  data: number[];
  timestamps?: number[];
  variant: 'cpu' | 'mem' | 'gpu';
  unit?: string;
  /** 像素高度；传 '100%' 让图表填满父容器（父级需有确定高度） */
  height?: number | string;
}

function fmtTime(ts: number) {
  return new Date(ts * 1000).toLocaleTimeString('zh-CN', {
    hour12: false,
    hour: '2-digit',
    minute: '2-digit',
    second: '2-digit',
  });
}

export function TrendChart({
  data,
  timestamps,
  variant,
  unit = '%',
  height = 80,
}: TrendChartProps) {
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

    const pal = chartPalette(theme);
    const tc = chartThemeColors(theme);
    const lineColor = variant === 'cpu' ? pal.accent : variant === 'mem' ? pal.success : pal.violet;
    const isDark = theme === 'dark';
    const maxVal = Math.max(...data, 1);
    const yMax =
      variant === 'gpu'
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
        backgroundColor: tc.tooltipBg,
        borderColor: tc.tooltipBorder,
        borderWidth: 1,
        padding: [8, 12],
        textStyle: {
          color: tc.tooltipText,
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
            <div><span style="color:${lineColor}">●</span> <b>${val}${unit}</b></div>
          </div>`;
        },
        axisPointer: {
          type: 'cross',
          crossStyle: { color: lineColor, opacity: 0.35 },
          lineStyle: { color: lineColor, opacity: 0.25, type: 'dashed' },
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
          lineStyle: { color: tc.split, type: 'dashed' },
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
              color: lineColor,
              borderColor: isDark ? '#18181b' : '#fff',
              borderWidth: 2,
              shadowBlur: 8,
              shadowColor: lineColor,
            },
          },
          lineStyle: { width: 2, color: lineColor, cap: 'round' },
          areaStyle: { color: areaGradient(lineColor, 0.2) },
        },
      ],
    };
  }, [data, timestamps, variant, unit, theme]);

  if (data.length < 2) {
    return (
      <div
        style={{ height }}
        className="flex h-full items-center justify-center rounded-lg border border-dashed border-default-200 font-mono text-[10px] opacity-35"
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

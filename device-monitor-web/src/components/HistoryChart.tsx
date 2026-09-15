import { useMemo } from 'react';
import ReactECharts from 'echarts-for-react';
import type { EChartsOption } from 'echarts';
import {
  areaGradient,
  chartPalette,
  chartThemeColors,
  fmtChartTime,
  useChartTheme,
} from './chartTheme';

export interface SeriesDef {
  name: string;
  data: number[];
  color: string;
  unit?: string;
  yAxisIndex?: 0 | 1;
  area?: boolean;
}

interface HistoryChartProps {
  timestamps: number[];
  series: SeriesDef[];
  range?: string;
  height?: number;
}

export function HistoryChart({
  timestamps,
  series,
  range = '1h',
  height = 220,
}: HistoryChartProps) {
  const theme = useChartTheme();
  const colors = chartThemeColors(theme);
  const palette = chartPalette(theme);

  const option = useMemo<EChartsOption>(() => {
    if (timestamps.length < 2 || series.every((s) => s.data.length < 2)) return {};

    const labels = timestamps.map((ts) => fmtChartTime(ts, range));
    const hasSecondAxis = series.some((s) => s.yAxisIndex === 1);

    return {
      animation: true,
      legend: {
        top: 0,
        right: 0,
        textStyle: { color: colors.text, fontSize: 10 },
        itemWidth: 12,
        itemHeight: 8,
      },
      grid: { left: 48, right: hasSecondAxis ? 48 : 16, top: 36, bottom: 48 },
      tooltip: {
        trigger: 'axis',
        backgroundColor: colors.tooltipBg,
        borderColor: colors.tooltipBorder,
        textStyle: {
          color: colors.tooltipText,
          fontSize: 11,
          fontFamily: 'ui-monospace, monospace',
        },
        axisPointer: { type: 'cross', label: { backgroundColor: colors.tooltipBg } },
      },
      dataZoom: [
        { type: 'inside', start: 0, end: 100 },
        {
          type: 'slider',
          start: 0,
          end: 100,
          height: 18,
          bottom: 4,
          borderColor: colors.axis,
          fillerColor: `${palette.accent}33`,
          handleStyle: { color: palette.accent },
          textStyle: { color: colors.text, fontSize: 9 },
        },
      ],
      xAxis: {
        type: 'category',
        data: labels,
        boundaryGap: false,
        axisLine: { lineStyle: { color: colors.axis } },
        axisLabel: { color: colors.text, fontSize: 9, hideOverlap: true },
      },
      yAxis: [
        {
          type: 'value',
          axisLine: { show: false },
          axisLabel: { color: colors.text, fontSize: 9 },
          splitLine: { lineStyle: { color: colors.split, type: 'dashed' } },
        },
        ...(hasSecondAxis
          ? [
              {
                type: 'value' as const,
                axisLine: { show: false },
                axisLabel: { color: colors.text, fontSize: 9 },
                splitLine: { show: false },
              },
            ]
          : []),
      ],
      series: series.map((s) => ({
        name: s.name,
        type: 'line' as const,
        data: s.data,
        yAxisIndex: s.yAxisIndex ?? 0,
        smooth: 0.3,
        showSymbol: false,
        lineStyle: { width: 2, color: s.color },
        itemStyle: { color: s.color },
        ...(s.area ? { areaStyle: { color: areaGradient(s.color, 0.2) } } : {}),
      })),
    };
  }, [timestamps, series, range, theme, colors, palette]);

  if (timestamps.length < 2) {
    return (
      <div
        style={{ height }}
        className="flex items-center justify-center rounded-xl border border-dashed border-default-200 text-[11px] font-mono opacity-40"
      >
        暂无历史数据
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

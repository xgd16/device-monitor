import { useMemo } from 'react';
import ReactECharts from 'echarts-for-react';
import type { EChartsOption } from 'echarts';
import { chartThemeColors, fmtChartTime, useChartTheme } from './chartTheme';

export interface SeriesDef {
  name: string;
  data: number[];
  color: string;
  unit?: string;
  yAxisIndex?: 0 | 1;
  area?: boolean;
}

interface HistoryChartProps {
  title: string;
  timestamps: number[];
  series: SeriesDef[];
  range?: string;
  height?: number;
  yAxisNames?: [string?, string?];
}

export function HistoryChart({
  title,
  timestamps,
  series,
  range = '1h',
  height = 220,
  yAxisNames,
}: HistoryChartProps) {
  const theme = useChartTheme();
  const colors = chartThemeColors(theme);

  const option = useMemo<EChartsOption>(() => {
    if (timestamps.length < 2 || series.every((s) => s.data.length < 2)) return {};

    const labels = timestamps.map((ts) => fmtChartTime(ts, range));
    const hasSecondAxis = series.some((s) => s.yAxisIndex === 1);

    return {
      animation: true,
      title: {
        text: title,
        left: 0,
        top: 0,
        textStyle: { color: colors.text, fontSize: 11, fontWeight: 500, fontFamily: 'ui-monospace, monospace' },
      },
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
        textStyle: { color: colors.tooltipText, fontSize: 11, fontFamily: 'ui-monospace, monospace' },
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
          fillerColor: theme === 'dark' ? 'rgba(56,189,248,0.15)' : 'rgba(56,189,248,0.2)',
          handleStyle: { color: '#38bdf8' },
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
          name: yAxisNames?.[0],
          nameTextStyle: { color: colors.text, fontSize: 9 },
          axisLine: { show: false },
          axisLabel: { color: colors.text, fontSize: 9 },
          splitLine: { lineStyle: { color: colors.split, type: 'dashed' } },
        },
        ...(hasSecondAxis
          ? [{
              type: 'value' as const,
              name: yAxisNames?.[1],
              nameTextStyle: { color: colors.text, fontSize: 9 },
              axisLine: { show: false },
              axisLabel: { color: colors.text, fontSize: 9 },
              splitLine: { show: false },
            }]
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
        ...(s.area
          ? {
              areaStyle: {
                color: {
                  type: 'linear' as const,
                  x: 0, y: 0, x2: 0, y2: 1,
                  colorStops: [
                    { offset: 0, color: s.color + '55' },
                    { offset: 1, color: s.color + '05' },
                  ],
                },
              },
            }
          : {}),
      })),
    };
  }, [title, timestamps, series, range, theme, colors, yAxisNames]);

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

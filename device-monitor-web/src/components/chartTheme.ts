import { useEffect, useState } from 'react';

export function useChartTheme() {
  const [theme, setTheme] = useState<'dark' | 'light'>('dark');

  useEffect(() => {
    const el = document.documentElement;
    const read = () => setTheme((el.getAttribute('data-theme') as 'dark' | 'light') || 'dark');
    read();
    const obs = new MutationObserver(read);
    obs.observe(el, { attributes: true, attributeFilter: ['data-theme'] });
    return () => obs.disconnect();
  }, []);

  return theme;
}

export function fmtChartTime(ts: number, range: string) {
  const d = new Date(ts * 1000);
  if (range === '7d') {
    return d.toLocaleString('zh-CN', {
      month: '2-digit',
      day: '2-digit',
      hour: '2-digit',
      minute: '2-digit',
      hour12: false,
    });
  }
  return d.toLocaleTimeString('zh-CN', {
    hour12: false,
    hour: '2-digit',
    minute: '2-digit',
    second: '2-digit',
  });
}

export function chartThemeColors(theme: 'dark' | 'light') {
  const isDark = theme === 'dark';
  return {
    text: isDark ? 'rgba(255,255,255,0.55)' : 'rgba(0,0,0,0.55)',
    axis: isDark ? 'rgba(255,255,255,0.12)' : 'rgba(0,0,0,0.12)',
    split: isDark ? 'rgba(255,255,255,0.06)' : 'rgba(0,0,0,0.06)',
    tooltipBg: isDark ? 'rgba(22,22,28,0.96)' : 'rgba(255,255,255,0.98)',
    tooltipBorder: isDark ? 'rgba(255,255,255,0.12)' : 'rgba(0,0,0,0.08)',
    tooltipText: isDark ? '#e4e4e7' : '#27272a',
  };
}

/**
 * 图表色板 —— 与 index.css 的主题令牌一一对应。
 * ECharts 画在 canvas 上，拿不到 CSS 变量，所以这里保留等价的十六进制值；
 * 改主题色时请同步这两处。
 */
export function chartPalette(theme: 'dark' | 'light') {
  return theme === 'dark'
    ? {
        accent: '#38d9d1',
        success: '#47d992',
        warning: '#f9b94b',
        danger: '#f66164',
        neutral: '#9399a0',
        violet: '#a29bff',
      }
    : {
        accent: '#007684',
        success: '#009259',
        warning: '#d07b18',
        danger: '#d33944',
        neutral: '#6c7278',
        violet: '#6356bf',
      };
}

/** 面积渐变：顶部淡染 → 底部透明 */
export function areaGradient(color: string, topAlpha = 0.22) {
  return {
    type: 'linear' as const,
    x: 0,
    y: 0,
    x2: 0,
    y2: 1,
    colorStops: [
      {
        offset: 0,
        color: `${color}${Math.round(topAlpha * 255)
          .toString(16)
          .padStart(2, '0')}`,
      },
      { offset: 1, color: `${color}00` },
    ],
  };
}

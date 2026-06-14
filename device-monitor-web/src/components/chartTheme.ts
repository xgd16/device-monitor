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
    return d.toLocaleString('zh-CN', { month: '2-digit', day: '2-digit', hour: '2-digit', minute: '2-digit', hour12: false });
  }
  return d.toLocaleTimeString('zh-CN', { hour12: false, hour: '2-digit', minute: '2-digit', second: '2-digit' });
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

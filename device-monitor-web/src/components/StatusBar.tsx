import { Chip } from '@heroui/react';
import { fmtUptime } from './utils';

interface StatusBarProps {
  connected: boolean;
  uptime: number;
  theme: 'dark' | 'light';
  onThemeChange: (theme: 'dark' | 'light') => void;
}

export function StatusBar({ connected, uptime, theme, onThemeChange }: StatusBarProps) {
  return (
    <div className="flex items-center justify-between px-3 py-2 sm:px-4 sm:py-2.5">
      <div className="flex items-center gap-2 sm:gap-3">
        <h1 className="text-sm sm:text-base font-semibold tracking-tight">设备监控</h1>
        <Chip size="sm" color="accent" variant="secondary">实时</Chip>
      </div>
      <div className="flex items-center gap-2 sm:gap-4 text-[10px] sm:text-xs font-mono opacity-60">
        <span className="hidden sm:inline">运行 {fmtUptime(uptime)}</span>
        <div className="flex items-center gap-1.5">
          <span
            className="inline-block w-[6px] h-[6px] rounded-full"
            style={{ background: connected ? 'var(--success)' : 'var(--danger)' }}
          />
          <span>{connected ? '已连接' : '断开'}</span>
        </div>
        <div
          className="flex items-center rounded-lg p-0.5 gap-0.5 bg-default-100 border border-default-200"
          role="group"
          aria-label="主题切换"
        >
          <button
            type="button"
            onClick={() => onThemeChange('light')}
            className={`px-2.5 py-1 rounded-md text-[10px] sm:text-[11px] font-medium transition-colors ${
              theme === 'light'
                ? 'bg-accent text-accent-foreground shadow-sm'
                : 'opacity-50 hover:opacity-80'
            }`}
          >
            浅色
          </button>
          <button
            type="button"
            onClick={() => onThemeChange('dark')}
            className={`px-2.5 py-1 rounded-md text-[10px] sm:text-[11px] font-medium transition-colors ${
              theme === 'dark'
                ? 'bg-accent text-accent-foreground shadow-sm'
                : 'opacity-50 hover:opacity-80'
            }`}
          >
            深色
          </button>
        </div>
      </div>
    </div>
  );
}

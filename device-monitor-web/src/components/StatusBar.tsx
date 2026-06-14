import { Chip, Switch } from '@heroui/react';
import { fmtUptime } from './utils';

interface StatusBarProps {
  connected: boolean;
  uptime: number;
  theme: 'dark' | 'light';
  onThemeToggle: () => void;
}

export function StatusBar({ connected, uptime, theme, onThemeToggle }: StatusBarProps) {
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
        <Switch
          size="sm"
          isSelected={theme === 'dark'}
          onChange={onThemeToggle}
          aria-label="切换主题"
        />
      </div>
    </div>
  );
}

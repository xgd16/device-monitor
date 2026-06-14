import { Chip } from '@heroui/react';
import { fmtUptime } from './utils';

export type AppPage = 'monitor' | 'terminal' | 'files';

const PAGE_LABELS: Record<AppPage, string> = {
  monitor: '监控',
  terminal: '终端',
  files: '文件',
};

interface StatusBarProps {
  connected: boolean;
  uptime: number;
  theme: 'dark' | 'light';
  page: AppPage;
  onThemeChange: (theme: 'dark' | 'light') => void;
  onPageChange: (page: AppPage) => void;
}

function NavGroup<T extends string>({
  value,
  options,
  labels,
  onChange,
  ariaLabel,
}: {
  value: T;
  options: readonly T[];
  labels: Record<T, string>;
  onChange: (v: T) => void;
  ariaLabel: string;
}) {
  return (
    <div
      className="flex items-center rounded-lg p-0.5 gap-0.5 bg-default-100 border border-default-200"
      role="group"
      aria-label={ariaLabel}
    >
      {options.map((opt) => (
        <button
          key={opt}
          type="button"
          onClick={() => onChange(opt)}
          className={`px-2 sm:px-2.5 py-1 rounded-md text-[10px] sm:text-[11px] font-medium transition-colors whitespace-nowrap ${
            value === opt
              ? 'bg-accent text-accent-foreground shadow-sm'
              : 'text-foreground/60 hover:text-foreground/90'
          }`}
        >
          {labels[opt]}
        </button>
      ))}
    </div>
  );
}

export function StatusBar({ connected, uptime, theme, page, onThemeChange, onPageChange }: StatusBarProps) {
  return (
    <div className="flex items-center justify-between gap-2 px-3 py-2 sm:px-4 sm:py-2.5 border-b border-default-200 shrink-0">
      <div className="flex items-center gap-2 sm:gap-3 min-w-0">
        <h1 className="text-sm sm:text-base font-semibold tracking-tight shrink-0">设备监控</h1>
        {page === 'monitor' && (
          <Chip size="sm" color="accent" variant="secondary" className="hidden xs:flex">实时</Chip>
        )}
        <NavGroup
          value={page}
          options={['monitor', 'terminal', 'files'] as const}
          labels={PAGE_LABELS}
          onChange={onPageChange}
          ariaLabel="页面切换"
        />
      </div>
      <div className="flex items-center gap-2 sm:gap-3 text-[10px] sm:text-xs font-mono text-foreground/60 shrink-0">
        {page === 'monitor' && (
          <span className="hidden lg:inline">运行 {fmtUptime(uptime)}</span>
        )}
        <div className="flex items-center gap-1.5">
          <span
            className="inline-block w-[6px] h-[6px] rounded-full"
            style={{ background: connected ? 'var(--success)' : 'var(--danger)' }}
          />
          <span className="hidden sm:inline">{connected ? '已连接' : '断开'}</span>
        </div>
        <NavGroup
          value={theme}
          options={['light', 'dark'] as const}
          labels={{ light: '浅色', dark: '深色' }}
          onChange={onThemeChange}
          ariaLabel="主题切换"
        />
      </div>
    </div>
  );
}

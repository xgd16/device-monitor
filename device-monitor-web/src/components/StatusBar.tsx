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
  /** 界面刷新间隔（秒）：1/3/5/10，驱动采集心跳、WS 推送与 DRM 屏 */
  refreshSecs: number;
  onRefreshChange: (secs: number) => void;
}

/** 分段开关：选中项用 accent 反白 */
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
      className="flex items-center gap-0.5 rounded-lg border border-default-200 bg-default-100 p-0.5"
      role="group"
      aria-label={ariaLabel}
    >
      {options.map((opt) => (
        <button
          key={opt}
          type="button"
          onClick={() => onChange(opt)}
          aria-pressed={value === opt}
          className={`rounded-md px-2 py-1 font-mono text-[10px] font-medium whitespace-nowrap transition-colors sm:px-2.5 sm:text-[11px] ${
            value === opt
              ? 'bg-accent text-accent-foreground shadow-sm'
              : 'text-foreground/55 hover:bg-default-200 hover:text-foreground/90'
          }`}
        >
          {labels[opt]}
        </button>
      ))}
    </div>
  );
}

/** 刷新间隔档位（秒），与后端 REFRESH_CHOICES 对齐 */
const REFRESH_OPTIONS = ['1', '3', '5', '10'] as const;
type RefreshOption = (typeof REFRESH_OPTIONS)[number];
const REFRESH_LABELS: Record<RefreshOption, string> = {
  '1': '1s',
  '3': '3s',
  '5': '5s',
  '10': '10s',
};

export function StatusBar({
  connected,
  uptime,
  theme,
  page,
  onThemeChange,
  onPageChange,
  refreshSecs,
  onRefreshChange,
}: StatusBarProps) {
  return (
    <header className="relative z-20 shrink-0 border-b border-default-200/70 bg-background">
      {/* 中轴对称的收边高光 */}
      <div
        aria-hidden
        className="pointer-events-none absolute inset-x-0 bottom-0 h-px"
        style={{
          background:
            'linear-gradient(to right, transparent, color-mix(in oklab, var(--accent) 45%, transparent), transparent)',
        }}
      />

      <div className="mx-auto grid w-full max-w-[1760px] grid-cols-[1fr_auto_1fr] items-center gap-2 px-3 py-2 sm:gap-4 sm:px-4 sm:py-2.5 lg:px-5">
        {/* 左：标识 */}
        <div className="flex min-w-0 items-center gap-2 sm:gap-3">
          <span
            aria-hidden
            className="size-2 shrink-0 rounded-[2px]"
            style={{
              background: 'var(--accent)',
              boxShadow: '0 0 8px color-mix(in oklab, var(--accent) 60%, transparent)',
            }}
          />
          <h1 className="shrink-0 text-sm font-semibold tracking-tight sm:text-base">设备监控</h1>
          <span className="dm-sub hidden lg:inline">Mi Mix 3</span>
          {page === 'monitor' && (
            <Chip size="sm" color="accent" variant="secondary" className="hidden xl:flex">
              实时
            </Chip>
          )}
        </div>

        {/* 中：页面切换（严格居中） */}
        <NavGroup
          value={page}
          options={['monitor', 'terminal', 'files'] as const}
          labels={PAGE_LABELS}
          onChange={onPageChange}
          ariaLabel="页面切换"
        />

        {/* 右：状态 */}
        <div className="flex min-w-0 items-center justify-end gap-2 font-mono text-[10px] text-foreground/60 sm:gap-3 sm:text-xs">
          {page === 'monitor' && <span className="hidden lg:inline">运行 {fmtUptime(uptime)}</span>}
          <span className="flex items-center gap-1.5">
            <span className={`dm-live ${connected ? '' : 'dm-live--down'}`} />
            <span className="hidden sm:inline">{connected ? '已连接' : '断开'}</span>
          </span>
          {(REFRESH_OPTIONS as readonly string[]).includes(String(refreshSecs)) && (
            <NavGroup
              value={String(refreshSecs) as RefreshOption}
              options={REFRESH_OPTIONS}
              labels={REFRESH_LABELS}
              onChange={(v) => onRefreshChange(Number(v))}
              ariaLabel="刷新间隔"
            />
          )}
          <NavGroup
            value={theme}
            options={['light', 'dark'] as const}
            labels={{ light: '浅色', dark: '深色' }}
            onChange={onThemeChange}
            ariaLabel="主题切换"
          />
        </div>
      </div>
    </header>
  );
}

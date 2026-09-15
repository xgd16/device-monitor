import { Chip } from '@heroui/react';
import { Panel, StatCell } from './Panel';
import type { AlertItem } from '../types';

interface AlertsCardProps {
  alerts: AlertItem[];
  className?: string;
}

const LEVELS = {
  error: { label: '错误', color: 'danger' },
  warning: { label: '警告', color: 'warning' },
  info: { label: '信息', color: 'accent' },
} as const;

function levelOf(level: string) {
  return LEVELS[level as keyof typeof LEVELS] ?? LEVELS.info;
}

function fmtTime(ts: number) {
  return new Date(ts * 1000).toLocaleTimeString('zh-CN', {
    hour12: false,
    hour: '2-digit',
    minute: '2-digit',
  });
}

/** 相对时间，让「最近」这个概念一眼可读 */
function fmtAgo(ts: number) {
  const diff = Math.max(0, Math.floor(Date.now() / 1000) - ts);
  if (diff < 60) return `${diff} 秒前`;
  if (diff < 3600) return `${Math.floor(diff / 60)} 分前`;
  if (diff < 86400) return `${Math.floor(diff / 3600)} 小时前`;
  return `${Math.floor(diff / 86400)} 天前`;
}

export function AlertsCard({ alerts, className }: AlertsCardProps) {
  const errors = alerts.filter((a) => a.level === 'error').length;
  const warnings = alerts.filter((a) => a.level === 'warning').length;
  const latest = alerts[0];

  return (
    <Panel
      label="最近告警"
      index={13}
      className={`h-full ${className ?? ''}`}
      hint={alerts.length > 0 ? <span>{alerts.length} 条</span> : undefined}
      bodyClassName="gap-3"
    >
      <div className="grid shrink-0 grid-cols-2 gap-2.5">
        <StatCell
          label="错误"
          value={errors}
          color={errors > 0 ? 'danger' : undefined}
          sub={`警告 ${warnings} 条`}
        />
        <StatCell
          label="最近一条"
          value={latest ? fmtAgo(latest.timestamp) : '—'}
          sub={latest ? `共 ${alerts.length} 条记录` : '系统安静'}
        />
      </div>

      {alerts.length === 0 ? (
        <div className="flex flex-1 items-center justify-center py-6 font-mono text-[11px] opacity-30">
          暂无告警
        </div>
      ) : (
        <div className="flex min-h-0 flex-1 flex-col content-start overflow-y-auto pr-1">
          {alerts.slice(0, 10).map((a) => {
            const lv = levelOf(a.level);
            return (
              <div key={a.id} className="dm-row flex items-start gap-2.5 py-2 first:pt-0">
                <Chip size="sm" color={lv.color} variant="secondary" className="shrink-0">
                  {lv.label}
                </Chip>
                <div className="flex min-w-0 flex-1 flex-col gap-0.5">
                  <span className="truncate font-mono text-[11px] font-medium xl:text-xs">
                    {a.title}
                  </span>
                  <span className="truncate text-[11px] opacity-50">{a.message}</span>
                </div>
                <span
                  className="shrink-0 font-mono text-[9px] opacity-35"
                  title={new Date(a.timestamp * 1000).toLocaleString('zh-CN')}
                >
                  {fmtTime(a.timestamp)}
                </span>
              </div>
            );
          })}
        </div>
      )}

      <div className="mt-auto flex flex-wrap items-center justify-center gap-x-2 border-t border-default-100 pt-2 font-mono text-[9px] opacity-40 xl:text-[10px]">
        <span>共 {alerts.length} 条</span>
        <span>·</span>
        <span>最多显示 10 条</span>
        {alerts.length > 0 && (
          <>
            <span>·</span>
            <span>警告及以上 {errors + warnings} 条</span>
          </>
        )}
      </div>
    </Panel>
  );
}

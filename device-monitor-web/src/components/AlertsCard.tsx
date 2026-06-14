import { Card, Chip } from '@heroui/react';
import type { AlertItem } from '../types';

interface AlertsCardProps {
  alerts: AlertItem[];
}

export function AlertsCard({ alerts }: AlertsCardProps) {
  if (alerts.length === 0) return null;

  return (
    <Card className="p-4 sm:p-5 flex flex-col gap-3">
      <div className="flex items-center justify-between">
        <span className="text-[10px] font-mono uppercase tracking-widest opacity-50">最近告警</span>
        <Chip size="sm" color="danger" variant="secondary">{alerts.length}</Chip>
      </div>
      <div className="flex flex-col gap-1.5 max-h-48 overflow-y-auto">
        {alerts.slice(0, 10).map(a => (
          <div
            key={a.id}
            className="flex items-center gap-2 sm:gap-3 py-1.5 px-2 rounded-lg bg-default-100"
          >
            <Chip
              size="sm"
              color={a.level === 'error' ? 'danger' : a.level === 'warning' ? 'warning' : 'accent'}
              variant="secondary"
            >
              {a.level === 'error' ? '错误' : a.level === 'warning' ? '警告' : '信息'}
            </Chip>
            <span className="font-mono text-[9px] sm:text-[10px] opacity-40 flex-shrink-0">
              {new Date(a.timestamp * 1000).toLocaleTimeString('zh-CN', { hour12: false })}
            </span>
            <span className="font-mono font-medium text-[11px] sm:text-[12px] flex-shrink-0">{a.title}</span>
            <span className="text-[11px] sm:text-[12px] truncate opacity-50">{a.message}</span>
          </div>
        ))}
      </div>
    </Card>
  );
}

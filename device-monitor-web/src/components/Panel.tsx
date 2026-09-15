import type { CSSProperties, ReactNode } from 'react';
import { Card } from '@heroui/react';

/** 面板：带刻线的仪表盘面。所有监控卡片共用同一副骨架。 */
export function Panel({
  label,
  hint,
  index = 0,
  className,
  bodyClassName = 'gap-3',
  flush = false,
  children,
}: {
  label: string;
  hint?: ReactNode;
  /** 入场动画的错峰序号 */
  index?: number;
  className?: string;
  bodyClassName?: string;
  /** 取消内边距，供表格/工具栏类面板贴边使用 */
  flush?: boolean;
  children: ReactNode;
}) {
  return (
    <Card className={`dm-panel ${className ?? ''}`} style={{ '--dm-i': index } as CSSProperties}>
      <div className="dm-panel__head">
        <span className="dm-panel__title">
          <span className="dm-label">{label}</span>
        </span>
        {hint != null && <div className="dm-panel__hint">{hint}</div>}
      </div>
      <div className={`dm-panel__body ${flush ? 'dm-panel__body--flush' : ''} ${bodyClassName}`}>
        {children}
      </div>
    </Card>
  );
}

const BAND_COLS = {
  1: 'grid-cols-1',
  2: 'grid-cols-1 md:grid-cols-2',
  3: 'grid-cols-1 md:grid-cols-3',
  4: 'grid-cols-2 lg:grid-cols-4',
} as const;

/**
 * 分区：一条沿中轴左右展开的图例线 + 一个等分网格。
 * cols 只用偶数或 3 —— 保证每一行都以页面中轴镜像对称。
 */
export function Band({
  index,
  title,
  cols,
  className,
  children,
}: {
  index: number;
  title: string;
  cols: keyof typeof BAND_COLS;
  className?: string;
  children: ReactNode;
}) {
  return (
    <section className={`dm-band ${className ?? ''}`} style={{ '--dm-i': index } as CSSProperties}>
      <div className="dm-band__rule">
        <span className="dm-band__index">{String(index).padStart(2, '0')}</span>
        <span className="dm-band__title">{title}</span>
      </div>
      <div className={`grid items-stretch gap-3 xl:gap-4 ${BAND_COLS[cols]}`}>{children}</div>
    </section>
  );
}

/** 面板页眉右侧的「标签 + 数值」小条目 */
export function HintStat({
  label,
  value,
  color,
}: {
  label?: string;
  value: ReactNode;
  color?: string;
}) {
  return (
    <span className="inline-flex items-baseline gap-1.5">
      {label && <span className="opacity-45">{label}</span>}
      <span style={color ? { color: `var(--${color})` } : undefined}>{value}</span>
    </span>
  );
}

/** 面板主数值：等宽、大字号、默认带蚀刻渐变（无 color 时） */
export function Hero({
  value,
  unit,
  note,
  color,
  className = 'text-3xl xl:text-[2.1rem]',
}: {
  value: ReactNode;
  unit?: string;
  note?: ReactNode;
  color?: string;
  className?: string;
}) {
  return (
    <div className="flex flex-wrap items-baseline gap-x-2 gap-y-0.5">
      <span
        className={`dm-hero font-light leading-none ${className} ${color ? '' : 'dm-hero--etch'}`}
        style={color ? { color: `var(--${color})` } : undefined}
      >
        {value}
        {unit && <span className="ml-0.5 text-[0.42em] opacity-55">{unit}</span>}
      </span>
      {note != null && <span className="font-mono text-[10px] opacity-45">{note}</span>}
    </div>
  );
}

/** 面板内的凹槽统计格。深度诊断区的两张卡片都用 2 格，形成对称的顶栏。 */
export function StatCell({
  label,
  value,
  unit,
  sub,
  color,
}: {
  label: string;
  value: ReactNode;
  unit?: string;
  sub?: ReactNode;
  color?: string;
}) {
  return (
    <div className="dm-inset flex flex-col gap-1 px-2.5 py-2">
      <span className="dm-sub">{label}</span>
      <span
        className="dm-hero text-xl font-light leading-none"
        style={color ? { color: `var(--${color})` } : undefined}
      >
        {value}
        {unit && <span className="ml-0.5 text-[0.5em] opacity-55">{unit}</span>}
      </span>
      {sub != null && <span className="font-mono text-[9px] opacity-35">{sub}</span>}
    </div>
  );
}

/** 指标行：名称 + 比例条 + 数值，行距与条宽全站一致 */
export function MeterRow({
  label,
  value,
  ratio,
  color,
  title,
}: {
  label: string;
  value: ReactNode;
  /** 0–100 */
  ratio: number;
  color?: string;
  title?: string;
}) {
  const pct = Math.max(0, Math.min(100, ratio));
  return (
    <div className="flex items-center gap-2 font-mono text-[10px] xl:text-[11px]" title={title}>
      <span className="min-w-0 flex-1 truncate opacity-55">{label}</span>
      <span className="h-1 w-12 shrink-0 overflow-hidden rounded-full bg-default-200 xl:w-20">
        <span
          className="block h-full rounded-full transition-[width] duration-500"
          style={{
            width: `${pct}%`,
            background: color ? `var(--${color})` : 'var(--accent)',
          }}
        />
      </span>
      <span
        className="w-12 shrink-0 text-right tabular-nums"
        style={color ? { color: `var(--${color})` } : undefined}
      >
        {value}
      </span>
    </div>
  );
}

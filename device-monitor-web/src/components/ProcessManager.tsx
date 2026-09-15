import { useMemo, useState, useCallback } from 'react';
import { Chip, Button } from '@heroui/react';
import { Panel } from './Panel';
import { statusLabel, statusColor, fmtMem } from './utils';
import { killProcess } from '../api';
import type { ProcessInfo } from '../types';

interface ProcessManagerProps {
  processes: ProcessInfo[];
  onRefresh?: () => void;
  /** 侧栏紧凑模式：固定行数、无分页 */
  compact?: boolean;
}

type SortKey = 'pid' | 'name' | 'status' | 'cpu_usage' | 'memory_mb' | 'threads' | 'ppid';
type SortDir = 'asc' | 'desc';

const SIGNALS = [
  { key: 'TERM', label: '终止 (SIGTERM)', variant: 'secondary' as const },
  { key: 'KILL', label: '强杀 (SIGKILL)', variant: 'danger' as const },
  { key: 'STOP', label: '暂停 (SIGSTOP)', variant: 'outline' as const },
  { key: 'CONT', label: '继续 (SIGCONT)', variant: 'secondary' as const },
];

const PAGE_SIZE = 30;
const COMPACT_ROWS = 10;

export function ProcessManager({ processes, onRefresh, compact = false }: ProcessManagerProps) {
  const [search, setSearch] = useState('');
  const [sortKey, setSortKey] = useState<SortKey>('cpu_usage');
  const [sortDir, setSortDir] = useState<SortDir>('desc');
  const [actionPid, setActionPid] = useState<number | null>(null);
  const [actionLoading, setActionLoading] = useState(false);
  const [actionMsg, setActionMsg] = useState<{ type: 'ok' | 'err'; text: string } | null>(null);
  const [page, setPage] = useState(1);

  const toggleSort = (key: SortKey) => {
    if (sortKey === key) {
      setSortDir((d) => (d === 'asc' ? 'desc' : 'asc'));
    } else {
      setSortKey(key);
      setSortDir(key === 'name' || key === 'status' ? 'asc' : 'desc');
    }
    setPage(1);
  };

  const filtered = useMemo(() => {
    let list = processes;
    if (search.trim()) {
      const q = search.trim().toLowerCase();
      list = list.filter(
        (p) =>
          p.name.toLowerCase().includes(q) ||
          String(p.pid).includes(q) ||
          String(p.ppid).includes(q),
      );
    }
    return [...list].sort((a, b) => {
      let va = a[sortKey];
      let vb = b[sortKey];
      if (typeof va === 'string') {
        va = va.toLowerCase() as any;
        vb = (vb as string).toLowerCase() as any;
      }
      if (va < vb) return sortDir === 'asc' ? -1 : 1;
      if (va > vb) return sortDir === 'asc' ? 1 : -1;
      return 0;
    });
  }, [processes, search, sortKey, sortDir]);

  // 搜索/排序变化时重置页码
  const totalPages = Math.max(1, Math.ceil(filtered.length / PAGE_SIZE));
  const safePage = Math.min(page, totalPages);
  const paged = compact
    ? filtered.slice(0, COMPACT_ROWS)
    : filtered.slice((safePage - 1) * PAGE_SIZE, safePage * PAGE_SIZE);

  const stats = useMemo(() => {
    const totalMem = processes.reduce((s, p) => s + p.memory_mb, 0);
    const totalCpu = processes.reduce((s, p) => s + p.cpu_usage, 0);
    const running = processes.filter((p) => p.status.includes('run')).length;
    return { totalMem, totalCpu, running, total: processes.length };
  }, [processes]);

  const handleSignal = useCallback(
    async (pid: number, signal: string) => {
      setActionLoading(true);
      setActionMsg(null);
      try {
        const res = await killProcess(pid, signal);
        if (res.code === 0) {
          setActionMsg({ type: 'ok', text: `已发送 ${signal} 到 PID ${pid}` });
          setTimeout(() => {
            onRefresh?.();
            setActionPid(null);
            setActionMsg(null);
          }, 800);
        } else {
          setActionMsg({ type: 'err', text: res.error || '操作失败' });
        }
      } catch {
        setActionMsg({ type: 'err', text: '请求失败' });
      } finally {
        setActionLoading(false);
      }
    },
    [onRefresh],
  );

  const SortIcon = ({ col }: { col: SortKey }) => {
    if (sortKey !== col) return <span className="opacity-20 ml-0.5">↕</span>;
    return <span className="opacity-60 ml-0.5">{sortDir === 'asc' ? '↑' : '↓'}</span>;
  };

  const ThSort = ({
    col,
    label,
    align = 'text-right',
    className = '',
  }: {
    col: SortKey;
    label: string;
    align?: 'text-right' | 'text-left';
    className?: string;
  }) => (
    <th
      className={`${align} cursor-pointer font-mono text-[10px] whitespace-nowrap select-none opacity-50 hover:opacity-80 sm:text-[11px] ${className}`}
      onClick={() => toggleSort(col)}
    >
      {label}
      <SortIcon col={col} />
    </th>
  );

  return (
    <Panel
      label={compact ? '进程 Top' : '进程管理'}
      index={22}
      hint={
        <>
          <span>{stats.total} 进程</span>
          <span>·</span>
          <span className="text-success">{stats.running} 运行</span>
          {!compact && (
            <>
              <span className="hidden sm:inline">·</span>
              <span className="hidden sm:inline">CPU {stats.totalCpu.toFixed(1)}%</span>
              <span className="hidden sm:inline">·</span>
              <span className="hidden sm:inline">内存 {fmtMem(stats.totalMem)}</span>
            </>
          )}
        </>
      }
      flush
    >
      {!compact && (
        <div className="flex items-center gap-2 px-4 pt-3 pb-3 sm:px-5">
          <input
            type="text"
            placeholder="搜索进程名、PID、PPID..."
            value={search}
            onChange={(e) => {
              setSearch(e.target.value);
              setPage(1);
            }}
            className="dm-input h-8 flex-1 py-0 text-xs"
          />
          <span className="shrink-0 font-mono text-[10px] whitespace-nowrap opacity-40">
            {search ? `匹配 ${filtered.length}` : '按列头排序'}
          </span>
        </div>
      )}

      {/* 表格 */}
      <div className="min-h-0 flex-1 overflow-x-auto">
        <table className="w-full text-left">
          <thead>
            <tr className="border-y border-default-100 bg-default-50">
              <ThSort col="pid" label="PID" className="pl-4 sm:pl-5" />
              <ThSort col="name" label="名称" align="text-left" className="w-full pl-2" />
              <ThSort col="status" label="状态" />
              <ThSort col="cpu_usage" label="CPU%" />
              <ThSort col="memory_mb" label="内存" />
              {!compact && <ThSort col="threads" label="线程" className="hidden sm:table-cell" />}
              {!compact && <ThSort col="ppid" label="PPID" className="hidden md:table-cell" />}
              {!compact && (
                <th className="pr-4 sm:pr-5 text-right font-mono text-[10px] whitespace-nowrap opacity-50">
                  操作
                </th>
              )}
            </tr>
          </thead>
          <tbody>
            {paged.map((p) => (
              <tr
                key={p.pid}
                className={`border-b border-default-100 transition-colors hover:bg-default-50 ${
                  actionPid === p.pid ? 'bg-default-100' : ''
                }`}
              >
                <td className="py-1.5 pl-4 text-right font-mono text-[10px] whitespace-nowrap opacity-40 sm:pl-5 sm:text-[11px]">
                  {p.pid}
                </td>
                <td className="py-1.5 pl-2 font-mono text-[11px] sm:text-[12px]" title={p.name}>
                  {p.name}
                </td>
                <td className="py-1.5 pr-2 text-right whitespace-nowrap">
                  <Chip size="sm" color={statusColor(p.status)} variant="secondary">
                    {statusLabel(p.status)}
                  </Chip>
                </td>
                <td
                  className="py-1.5 pr-2 text-right font-mono text-[10px] tabular-nums whitespace-nowrap sm:text-[11px]"
                  style={{
                    color:
                      p.cpu_usage > 50
                        ? 'var(--danger)'
                        : p.cpu_usage > 20
                          ? 'var(--warning)'
                          : undefined,
                  }}
                >
                  {p.cpu_usage.toFixed(1)}%
                </td>
                <td
                  className="py-1.5 pr-2 text-right font-mono text-[10px] tabular-nums whitespace-nowrap sm:text-[11px]"
                  style={{ color: p.memory_mb > 500 ? 'var(--warning)' : undefined }}
                >
                  {fmtMem(p.memory_mb)}
                </td>
                {!compact && (
                  <>
                    <td className="hidden py-1.5 pr-2 text-right font-mono text-[10px] whitespace-nowrap opacity-40 sm:table-cell sm:text-[11px]">
                      {p.threads}
                    </td>
                    <td className="hidden py-1.5 pr-2 text-right font-mono text-[10px] whitespace-nowrap opacity-30 md:table-cell sm:text-[11px]">
                      {p.ppid}
                    </td>
                    <td className="py-1.5 pr-4 text-right whitespace-nowrap sm:pr-5">
                      {actionPid === p.pid ? (
                        <div className="flex items-center justify-end gap-1">
                          {SIGNALS.map((s) => (
                            <Button
                              key={s.key}
                              size="sm"
                              variant={s.variant}
                              isDisabled={actionLoading}
                              onPress={() => handleSignal(p.pid, s.key)}
                              className="h-6 min-w-0 px-1.5 font-mono text-[9px]"
                            >
                              {s.key}
                            </Button>
                          ))}
                          <Button
                            size="sm"
                            variant="ghost"
                            onPress={() => {
                              setActionPid(null);
                              setActionMsg(null);
                            }}
                            className="h-6 min-w-0 px-1.5 font-mono text-[9px]"
                          >
                            ✕
                          </Button>
                        </div>
                      ) : (
                        <Button
                          size="sm"
                          variant="ghost"
                          onPress={() => {
                            setActionPid(p.pid);
                            setActionMsg(null);
                          }}
                          className="h-6 min-w-0 px-2 font-mono text-[10px] opacity-40 hover:opacity-100"
                        >
                          操作
                        </Button>
                      )}
                    </td>
                  </>
                )}
              </tr>
            ))}
            {filtered.length === 0 && (
              <tr>
                <td
                  colSpan={compact ? 5 : 8}
                  className="py-6 text-center font-mono text-[11px] opacity-30"
                >
                  {search ? '无匹配进程' : '无进程数据'}
                </td>
              </tr>
            )}
          </tbody>
        </table>
      </div>

      {/* 分页 */}
      {!compact && totalPages > 1 && (
        <div className="flex items-center justify-between border-t border-default-100 px-4 py-2 sm:px-5">
          <span className="font-mono text-[10px] opacity-30">
            {filtered.length} 结果 · 第 {safePage}/{totalPages} 页
          </span>
          <div className="flex items-center gap-1">
            <Button
              size="sm"
              variant="ghost"
              isDisabled={safePage <= 1}
              onPress={() => setPage(1)}
              className="h-6 min-w-0 px-1.5 font-mono text-[10px]"
            >
              «
            </Button>
            <Button
              size="sm"
              variant="ghost"
              isDisabled={safePage <= 1}
              onPress={() => setPage((p) => Math.max(1, p - 1))}
              className="h-6 min-w-0 px-1.5 font-mono text-[10px]"
            >
              ‹
            </Button>
            {(() => {
              const pages: number[] = [];
              const start = Math.max(1, safePage - 2);
              const end = Math.min(totalPages, safePage + 2);
              for (let i = start; i <= end; i++) pages.push(i);
              return pages.map((i) => (
                <Button
                  key={i}
                  size="sm"
                  variant={i === safePage ? 'secondary' : 'ghost'}
                  onPress={() => setPage(i)}
                  className="h-6 min-w-0 px-2 font-mono text-[10px]"
                >
                  {i}
                </Button>
              ));
            })()}
            <Button
              size="sm"
              variant="ghost"
              isDisabled={safePage >= totalPages}
              onPress={() => setPage((p) => Math.min(totalPages, p + 1))}
              className="h-6 min-w-0 px-1.5 font-mono text-[10px]"
            >
              ›
            </Button>
            <Button
              size="sm"
              variant="ghost"
              isDisabled={safePage >= totalPages}
              onPress={() => setPage(totalPages)}
              className="h-6 min-w-0 px-1.5 font-mono text-[10px]"
            >
              »
            </Button>
          </div>
        </div>
      )}

      {/* 操作反馈 */}
      {actionMsg && (
        <div
          className={`border-t border-default-200 px-4 py-2 font-mono text-[10px] sm:px-5 sm:text-[11px] ${
            actionMsg.type === 'ok' ? 'text-success' : 'text-danger'
          }`}
        >
          {actionMsg.text}
        </div>
      )}
    </Panel>
  );
}

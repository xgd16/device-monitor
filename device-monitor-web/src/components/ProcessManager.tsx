import { useMemo, useState, useCallback } from 'react';
import { Card, Chip, Button } from '@heroui/react';
import { statusLabel, statusColor } from './utils';
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
      setSortDir(d => d === 'asc' ? 'desc' : 'asc');
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
      list = list.filter(p =>
        p.name.toLowerCase().includes(q) ||
        String(p.pid).includes(q) ||
        String(p.ppid).includes(q)
      );
    }
    return [...list].sort((a, b) => {
      let va = a[sortKey];
      let vb = b[sortKey];
      if (typeof va === 'string') { va = va.toLowerCase() as any; vb = (vb as string).toLowerCase() as any; }
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
    const running = processes.filter(p => p.status.includes('run')).length;
    return { totalMem, totalCpu, running, total: processes.length };
  }, [processes]);

  const handleSignal = useCallback(async (pid: number, signal: string) => {
    setActionLoading(true);
    setActionMsg(null);
    try {
      const res = await killProcess(pid, signal);
      if (res.code === 0) {
        setActionMsg({ type: 'ok', text: `已发送 ${signal} 到 PID ${pid}` });
        setTimeout(() => { onRefresh?.(); setActionPid(null); setActionMsg(null); }, 800);
      } else {
        setActionMsg({ type: 'err', text: res.error || '操作失败' });
      }
    } catch {
      setActionMsg({ type: 'err', text: '请求失败' });
    } finally {
      setActionLoading(false);
    }
  }, [onRefresh]);

  const SortIcon = ({ col }: { col: SortKey }) => {
    if (sortKey !== col) return <span className="opacity-20 ml-0.5">↕</span>;
    return <span className="opacity-60 ml-0.5">{sortDir === 'asc' ? '↑' : '↓'}</span>;
  };

  const ThSort = ({ col, label, className = '' }: { col: SortKey; label: string; className?: string }) => (
    <th
      className={`text-right font-mono text-[10px] sm:text-[11px] opacity-50 cursor-pointer hover:opacity-80 select-none whitespace-nowrap ${className}`}
      onClick={() => toggleSort(col)}
    >
      {label}<SortIcon col={col} />
    </th>
  );

  return (
    <Card>
      {/* 头部 */}
      <div className={`px-4 sm:px-5 pt-3 sm:pt-4 pb-1 sm:pb-2 flex flex-col gap-2 ${compact ? 'pb-2' : ''}`}>
        <div className="flex items-center justify-between">
          <span className="text-[10px] font-mono uppercase tracking-widest opacity-50">
            {compact ? '进程 Top' : '进程管理'}
          </span>
          <div className="flex items-center gap-2 font-mono text-[10px] opacity-30">
            <span>{stats.total} 进程</span>
            <span>·</span>
            <span className="text-success">{stats.running} 运行</span>
            {!compact && (
              <>
                <span>·</span>
                <span className="hidden sm:inline">CPU {stats.totalCpu.toFixed(1)}%</span>
                <span className="hidden sm:inline">·</span>
                <span className="hidden sm:inline">内存 {stats.totalMem} MB</span>
              </>
            )}
          </div>
        </div>
        {!compact && (
        <div className="flex items-center gap-2">
          <input
            type="text"
            placeholder="搜索进程名、PID、PPID..."
            value={search}
            onChange={e => { setSearch(e.target.value); setPage(1); }}
            className="flex-1 bg-default-100 rounded-md px-3 py-1.5 text-xs font-mono outline-none focus:ring-1 focus:ring-accent placeholder:opacity-30"
          />
          {search && (
            <span className="font-mono text-[10px] opacity-30 whitespace-nowrap">
              匹配 {filtered.length}
            </span>
          )}
        </div>
        )}
      </div>

      {/* 表格 */}
      <div className="overflow-x-auto">
        <table className="w-full text-left">
          <thead>
            <tr className="border-b border-default-200">
              <ThSort col="pid" label="PID" className="text-right pl-4 sm:pl-5" />
              <ThSort col="name" label="名称" className="text-left pl-2" />
              <ThSort col="status" label="状态" className="text-right" />
              <ThSort col="cpu_usage" label="CPU%" className="text-right" />
              <ThSort col="memory_mb" label="内存" className="text-right" />
              {!compact && <ThSort col="threads" label="线程" className="text-right hidden sm:table-cell" />}
              {!compact && <ThSort col="ppid" label="PPID" className="text-right hidden md:table-cell" />}
              {!compact && <th className="text-right font-mono text-[10px] opacity-50 pr-4 sm:pr-5 whitespace-nowrap">操作</th>}
            </tr>
          </thead>
          <tbody>
            {paged.map(p => (
              <tr
                key={p.pid}
                className={`border-b border-default-100 hover:bg-default-50 transition-colors ${actionPid === p.pid ? 'bg-default-100' : ''}`}
              >
                <td className="text-right font-mono text-[10px] sm:text-[11px] opacity-40 pl-4 sm:pl-5 py-1.5">{p.pid}</td>
                <td className="font-mono text-[11px] sm:text-[12px] max-w-[100px] sm:max-w-[180px] truncate pl-2 py-1.5" title={p.name}>{p.name}</td>
                <td className="text-right py-1.5 pr-2">
                  <Chip size="sm" color={statusColor(p.status)} variant="secondary">{statusLabel(p.status)}</Chip>
                </td>
                <td
                  className="text-right font-mono text-[10px] sm:text-[11px] py-1.5 pr-2"
                  style={{ color: p.cpu_usage > 50 ? 'var(--danger)' : p.cpu_usage > 20 ? 'var(--warning)' : undefined }}
                >
                  {p.cpu_usage.toFixed(1)}%
                </td>
                <td
                  className="text-right font-mono text-[10px] sm:text-[11px] py-1.5 pr-2"
                  style={{ color: p.memory_mb > 500 ? 'var(--warning)' : undefined }}
                >
                  {p.memory_mb >= 1024 ? `${(p.memory_mb / 1024).toFixed(1)}G` : `${p.memory_mb}M`}
                </td>
                {!compact && (
                  <>
                <td className="text-right font-mono text-[10px] sm:text-[11px] opacity-40 py-1.5 pr-2 hidden sm:table-cell">{p.threads}</td>
                <td className="text-right font-mono text-[10px] sm:text-[11px] opacity-30 py-1.5 pr-2 hidden md:table-cell">{p.ppid}</td>
                <td className="text-right py-1.5 pr-4 sm:pr-5">
                  {actionPid === p.pid ? (
                    <div className="flex items-center gap-1 justify-end">
                      {SIGNALS.map(s => (
                        <Button
                          key={s.key}
                          size="sm"
                          variant={s.variant}
                          isDisabled={actionLoading}
                          onPress={() => handleSignal(p.pid, s.key)}
                          className="min-w-0 px-1.5 h-6 text-[9px] font-mono"
                        >
                          {s.key}
                        </Button>
                      ))}
                      <Button
                        size="sm"
                        variant="ghost"
                        onPress={() => { setActionPid(null); setActionMsg(null); }}
                        className="min-w-0 px-1.5 h-6 text-[9px] font-mono"
                      >
                        ✕
                      </Button>
                    </div>
                  ) : (
                    <Button
                      size="sm"
                      variant="ghost"
                      onPress={() => { setActionPid(p.pid); setActionMsg(null); }}
                      className="min-w-0 px-2 h-6 text-[10px] font-mono opacity-40 hover:opacity-100"
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
                <td colSpan={compact ? 5 : 8} className="text-center font-mono text-[11px] opacity-30 py-6">
                  {search ? '无匹配进程' : '无进程数据'}
                </td>
              </tr>
            )}
          </tbody>
        </table>
      </div>

      {/* 分页 */}
      {!compact && totalPages > 1 && (
        <div className="px-4 sm:px-5 py-2 flex items-center justify-between border-t border-default-200">
          <span className="font-mono text-[10px] opacity-30">
            {filtered.length} 结果 · 第 {safePage}/{totalPages} 页
          </span>
          <div className="flex items-center gap-1">
            <Button
              size="sm"
              variant="ghost"
              isDisabled={safePage <= 1}
              onPress={() => setPage(1)}
              className="min-w-0 px-1.5 h-6 text-[10px] font-mono"
            >
              «
            </Button>
            <Button
              size="sm"
              variant="ghost"
              isDisabled={safePage <= 1}
              onPress={() => setPage(p => Math.max(1, p - 1))}
              className="min-w-0 px-1.5 h-6 text-[10px] font-mono"
            >
              ‹
            </Button>
            {(() => {
              const pages: number[] = [];
              const start = Math.max(1, safePage - 2);
              const end = Math.min(totalPages, safePage + 2);
              for (let i = start; i <= end; i++) pages.push(i);
              return pages.map(i => (
                <Button
                  key={i}
                  size="sm"
                  variant={i === safePage ? 'secondary' : 'ghost'}
                  onPress={() => setPage(i)}
                  className="min-w-0 px-2 h-6 text-[10px] font-mono"
                >
                  {i}
                </Button>
              ));
            })()}
            <Button
              size="sm"
              variant="ghost"
              isDisabled={safePage >= totalPages}
              onPress={() => setPage(p => Math.min(totalPages, p + 1))}
              className="min-w-0 px-1.5 h-6 text-[10px] font-mono"
            >
              ›
            </Button>
            <Button
              size="sm"
              variant="ghost"
              isDisabled={safePage >= totalPages}
              onPress={() => setPage(totalPages)}
              className="min-w-0 px-1.5 h-6 text-[10px] font-mono"
            >
              »
            </Button>
          </div>
        </div>
      )}

      {/* 操作反馈 */}
      {actionMsg && (
        <div className={`px-4 sm:px-5 py-2 font-mono text-[10px] sm:text-[11px] border-t border-default-200 ${actionMsg.type === 'ok' ? 'text-success' : 'text-danger'}`}>
          {actionMsg.text}
        </div>
      )}
    </Card>
  );
}

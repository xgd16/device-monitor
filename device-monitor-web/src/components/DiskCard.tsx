import { useState, useEffect, useMemo } from 'react';
import { Card, ProgressBar, Chip } from '@heroui/react';
import { fetchDisk } from '../api';
import { percentColor } from './utils';

interface DiskInfo {
  device: string;
  mount: string;
  fstype: string;
  total_mb: number;
  used_mb: number;
  available_mb: number;
  usage_percent: number;
  inode_total: number;
  inode_used: number;
  inode_free: number;
  inode_percent: number;
  read_sectors: number;
  write_sectors: number;
  io_ticks_ms: number;
  disk_type: string;
}

function fmtSize(mb: number): string {
  if (mb >= 1024 * 1024) return `${(mb / 1024 / 1024).toFixed(1)} TB`;
  if (mb >= 1024) return `${(mb / 1024).toFixed(1)} GB`;
  return `${mb} MB`;
}

function fmtSectors(s: number): string {
  // 1 sector = 512 bytes
  const bytes = s * 512;
  if (bytes >= 1073741824) return `${(bytes / 1073741824).toFixed(1)} GB`;
  if (bytes >= 1048576) return `${(bytes / 1048576).toFixed(1)} MB`;
  if (bytes >= 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${bytes} B`;
}

function fmtInode(n: number): string {
  if (n >= 1000000) return `${(n / 1000000).toFixed(1)}M`;
  if (n >= 1000) return `${(n / 1000).toFixed(1)}K`;
  return `${n}`;
}

// 简单的水平条形图组件
function MiniBar({ value, max, color }: { value: number; max: number; color: string }) {
  const pct = max > 0 ? Math.min((value / max) * 100, 100) : 0;
  return (
    <div className="flex items-center gap-1.5">
      <div className="flex-1 h-1.5 bg-default-100 rounded-full overflow-hidden">
        <div className="h-full rounded-full transition-all" style={{ width: `${pct}%`, background: color }} />
      </div>
      <span className="font-mono text-[9px] opacity-40 w-8 text-right">{pct.toFixed(0)}%</span>
    </div>
  );
}

export function DiskCard() {
  const [disks, setDisks] = useState<DiskInfo[]>([]);

  useEffect(() => {
    const load = () => fetchDisk().then(setDisks).catch(() => {});
    load();
    const t = setInterval(load, 30000);
    return () => clearInterval(t);
  }, []);

  const summary = useMemo(() => {
    const total = disks.reduce((s, d) => s + d.total_mb, 0);
    const used = disks.reduce((s, d) => s + d.used_mb, 0);
    const avail = disks.reduce((s, d) => s + d.available_mb, 0);
    const readTotal = disks.reduce((s, d) => s + d.read_sectors, 0);
    const writeTotal = disks.reduce((s, d) => s + d.write_sectors, 0);
    return { total, used, avail, pct: total > 0 ? (used / total * 100) : 0, readTotal, writeTotal };
  }, [disks]);

  if (disks.length === 0) return null;

  return (
    <Card className="p-4 sm:p-5 flex flex-col gap-3">
      {/* 标题 */}
      <div className="flex items-center justify-between">
        <span className="text-[10px] font-mono uppercase tracking-widest opacity-50">磁盘</span>
        <div className="flex items-center gap-2 font-mono text-[10px] opacity-30">
          <span>{disks.length} 分区</span>
          <span>·</span>
          <span>{disks[0]?.disk_type || '未知'}</span>
        </div>
      </div>

      {/* 汇总 */}
      {disks.length > 1 && (
        <div className="flex items-center gap-3">
          <div className="flex-1">
            <ProgressBar value={summary.pct} size="sm" color={percentColor(summary.pct) as any}>
              <ProgressBar.Track>
                <ProgressBar.Fill />
              </ProgressBar.Track>
            </ProgressBar>
          </div>
          <span className="font-mono text-xs" style={{ color: `var(--${percentColor(summary.pct)})` }}>{summary.pct.toFixed(0)}%</span>
        </div>
      )}

      {/* 各分区详情 */}
      <div className="flex flex-col gap-3">
        {disks.map(d => (
          <div key={d.device} className="flex flex-col gap-2 py-2 border-b border-default-100 last:border-0">
            {/* 挂载点行 */}
            <div className="flex items-center gap-2">
              <span className="font-mono text-sm font-medium">{d.mount}</span>
              <Chip size="sm" variant="secondary" className="text-[9px]">{d.fstype}</Chip>
              <span className="font-mono text-[9px] opacity-25 ml-auto">{d.device}</span>
            </div>

            {/* 空间使用 */}
            <div className="flex flex-col gap-1">
              <div className="flex items-center justify-between">
                <span className="font-mono text-[10px] opacity-40">空间</span>
                <span className="font-mono text-[10px] opacity-40">
                  {fmtSize(d.used_mb)} / {fmtSize(d.total_mb)}
                </span>
              </div>
              <MiniBar value={d.used_mb} max={d.total_mb} color={`var(--${percentColor(d.usage_percent)})`} />
            </div>

            {/* inode 使用 */}
            <div className="flex flex-col gap-1">
              <div className="flex items-center justify-between">
                <span className="font-mono text-[10px] opacity-40">Inode</span>
                <span className="font-mono text-[10px] opacity-40">
                  {fmtInode(d.inode_used)} / {fmtInode(d.inode_total)}
                </span>
              </div>
              <MiniBar value={d.inode_used} max={d.inode_total} color={`var(--${percentColor(d.inode_percent)})`} />
            </div>

            {/* 详细数值 */}
            <div className="flex flex-wrap gap-x-4 gap-y-0.5 font-mono text-[10px] sm:text-[11px]">
              <span className="opacity-50">可用 <span className="text-success">{fmtSize(d.available_mb)}</span></span>
              <span style={{ color: `var(--${percentColor(d.usage_percent)})` }}>{d.usage_percent.toFixed(1)}%</span>
              <span className="opacity-30">读 {fmtSectors(d.read_sectors)}</span>
              <span className="opacity-30">写 {fmtSectors(d.write_sectors)}</span>
            </div>
          </div>
        ))}
      </div>

      {/* 总 I/O */}
      <div className="flex flex-wrap gap-x-4 gap-y-1 font-mono text-[10px] opacity-30 pt-1 border-t border-default-100">
        <span>总读取 {fmtSectors(summary.readTotal)}</span>
        <span>总写入 {fmtSectors(summary.writeTotal)}</span>
      </div>
    </Card>
  );
}

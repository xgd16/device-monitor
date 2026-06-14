import { useState, useEffect, useCallback, useMemo, useRef } from 'react';
import { Card, Button, Chip, Switch } from '@heroui/react';
import type { FileEntry } from '../types';
import {
  listFiles, readFile, writeFile, uploadFiles, downloadUrl,
  mkdir, renameFile, moveFile, copyFile, deleteFile,
  compressFiles, extractFiles, type ArchiveFormat,
} from '../api';

const QUICK_PATHS = ['/', '/data', '/sdcard', '/tmp', '/var/log'];

type SortKey = 'name' | 'size' | 'modified' | 'mode';
type SortDir = 'asc' | 'desc';

function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
  return `${(bytes / 1024 / 1024 / 1024).toFixed(2)} GB`;
}

function formatTime(ts: number): string {
  if (!ts) return '-';
  return new Date(ts * 1000).toLocaleString('zh-CN', { hour12: false });
}

function fileIcon(entry: FileEntry): string {
  if (entry.is_symlink) return '🔗';
  if (entry.is_dir) return '📁';
  const ext = entry.name.split('.').pop()?.toLowerCase();
  if (['zip', '7z', 'rar'].includes(ext ?? '')) return '📦';
  if (['jpg', 'jpeg', 'png', 'gif', 'webp'].includes(ext ?? '')) return '🖼';
  if (['mp4', 'mkv', 'avi'].includes(ext ?? '')) return '🎬';
  if (['sh', 'py', 'rs', 'js', 'ts', 'json', 'yaml', 'toml'].includes(ext ?? '')) return '📄';
  return '📄';
}

function isArchiveName(name: string): boolean {
  const ext = name.split('.').pop()?.toLowerCase();
  return ext === 'zip' || ext === '7z' || ext === 'rar';
}

interface ModalProps {
  open: boolean;
  title: string;
  onClose: () => void;
  onConfirm: () => void;
  confirmLabel?: string;
  danger?: boolean;
  children: React.ReactNode;
}

function SimpleModal({ open, title, onClose, onConfirm, confirmLabel = '确定', danger, children }: ModalProps) {
  if (!open) return null;
  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/40 p-4" onClick={onClose}>
      <div onClick={(e) => e.stopPropagation()}>
        <Card className="p-4 w-full max-w-md shadow-lg border border-default-200">
        <h3 className="text-sm font-semibold mb-3">{title}</h3>
        {children}
        <div className="flex justify-end gap-2 mt-4">
          <Button size="sm" variant="ghost" onPress={onClose}>取消</Button>
          <Button size="sm" variant={danger ? 'danger' : 'secondary'} onPress={onConfirm}>{confirmLabel}</Button>
        </div>
        </Card>
      </div>
    </div>
  );
}

interface FileManagerProps {
  fullPage?: boolean;
}

export function FileManager({ fullPage = false }: FileManagerProps) {
  const [currentPath, setCurrentPath] = useState('/');
  const [entries, setEntries] = useState<FileEntry[]>([]);
  const [parent, setParent] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [showHidden, setShowHidden] = useState(false);
  const [sortKey, setSortKey] = useState<SortKey>('name');
  const [sortDir, setSortDir] = useState<SortDir>('asc');
  const [selected, setSelected] = useState<FileEntry | null>(null);
  const [checkedPaths, setCheckedPaths] = useState<Set<string>>(new Set());

  // Modals
  const [mkdirOpen, setMkdirOpen] = useState(false);
  const [mkdirName, setMkdirName] = useState('');
  const [renameOpen, setRenameOpen] = useState(false);
  const [renameName, setRenameName] = useState('');
  const [moveOpen, setMoveOpen] = useState(false);
  const [moveTarget, setMoveTarget] = useState('');
  const [moveMode, setMoveMode] = useState<'move' | 'copy'>('move');
  const [deleteOpen, setDeleteOpen] = useState(false);
  const [deleteRecursive, setDeleteRecursive] = useState(false);
  const [compressOpen, setCompressOpen] = useState(false);
  const [compressFormat, setCompressFormat] = useState<ArchiveFormat>('zip');
  const [compressOutput, setCompressOutput] = useState('');
  const [extractOpen, setExtractOpen] = useState(false);
  const [extractDest, setExtractDest] = useState('');
  const [extractOverwrite, setExtractOverwrite] = useState(false);

  // Preview/editor drawer
  const [previewOpen, setPreviewOpen] = useState(false);
  const [previewContent, setPreviewContent] = useState('');
  const [previewBinary, setPreviewBinary] = useState(false);
  const [previewPath, setPreviewPath] = useState('');
  const [previewSaving, setPreviewSaving] = useState(false);

  const fileInputRef = useRef<HTMLInputElement>(null);
  const [dragOver, setDragOver] = useState(false);

  const load = useCallback(async (path: string) => {
    setLoading(true);
    setError(null);
    try {
      const data = await listFiles(path);
      setCurrentPath(data.path);
      setParent(data.parent);
      setEntries(data.entries);
    } catch (e: unknown) {
      const msg = (e as { response?: { data?: { error?: string } } })?.response?.data?.error ?? '加载失败';
      setError(msg);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    load(currentPath);
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  const navigate = (path: string) => {
    setSelected(null);
    setCheckedPaths(new Set());
    load(path);
  };

  const toggleCheck = (path: string) => {
    setCheckedPaths((prev) => {
      const next = new Set(prev);
      if (next.has(path)) next.delete(path);
      else next.add(path);
      return next;
    });
  };

  const compressTargets = useMemo(() => {
    if (checkedPaths.size > 0) {
      return entries.filter((e) => checkedPaths.has(e.path)).map((e) => e.path);
    }
    if (selected) return [selected.path];
    return [];
  }, [checkedPaths, selected, entries]);

  const openCompressModal = () => {
    const base = currentPath === '/' ? '/archive' : `${currentPath}/archive`;
    setCompressOutput(`${base}.${compressFormat}`);
    setCompressOpen(true);
  };

  const handleCompress = async () => {
    if (compressTargets.length === 0 || !compressOutput.trim()) return;
    setLoading(true);
    setError(null);
    try {
      await compressFiles(compressTargets, compressOutput.trim(), compressFormat);
      setCompressOpen(false);
      setCheckedPaths(new Set());
      load(currentPath);
    } catch (e: unknown) {
      const msg = (e as { response?: { data?: { error?: string } } })?.response?.data?.error ?? '压缩失败';
      setError(msg);
    } finally {
      setLoading(false);
    }
  };

  const openExtractModal = (entry: FileEntry) => {
    const base = entry.name.replace(/\.(zip|7z|rar)$/i, '');
    const dest = currentPath === '/' ? `/${base}` : `${currentPath}/${base}`;
    setExtractDest(dest);
    setExtractOverwrite(false);
    setExtractOpen(true);
  };

  const handleExtract = async () => {
    if (!selected || !extractDest.trim()) return;
    setLoading(true);
    setError(null);
    try {
      await extractFiles(selected.path, extractDest.trim(), extractOverwrite);
      setExtractOpen(false);
      load(currentPath);
    } catch (e: unknown) {
      const msg = (e as { response?: { data?: { error?: string } } })?.response?.data?.error ?? '解压失败';
      setError(msg);
    } finally {
      setLoading(false);
    }
  };

  const breadcrumbs = useMemo(() => {
    if (currentPath === '/') return [{ label: '/', path: '/' }];
    const parts = currentPath.split('/').filter(Boolean);
    const crumbs = [{ label: '/', path: '/' }];
    let acc = '';
    for (const p of parts) {
      acc += `/${p}`;
      crumbs.push({ label: p, path: acc });
    }
    return crumbs;
  }, [currentPath]);

  const filtered = useMemo(() => {
    let list = entries;
    if (!showHidden) {
      list = list.filter((e) => !e.name.startsWith('.'));
    }
    return [...list].sort((a, b) => {
      if (a.is_dir !== b.is_dir) return a.is_dir ? -1 : 1;
      let va: string | number = a[sortKey];
      let vb: string | number = b[sortKey];
      if (sortKey === 'name') {
        va = a.name.toLowerCase();
        vb = b.name.toLowerCase();
      }
      if (va < vb) return sortDir === 'asc' ? -1 : 1;
      if (va > vb) return sortDir === 'asc' ? 1 : -1;
      return 0;
    });
  }, [entries, showHidden, sortKey, sortDir]);

  const toggleSort = (key: SortKey) => {
    if (sortKey === key) {
      setSortDir((d) => (d === 'asc' ? 'desc' : 'asc'));
    } else {
      setSortKey(key);
      setSortDir(key === 'name' ? 'asc' : 'desc');
    }
  };

  const handleEntryClick = (entry: FileEntry) => {
    setSelected(entry);
  };

  const handleEntryDoubleClick = async (entry: FileEntry) => {
    if (entry.is_dir) {
      navigate(entry.path);
      return;
    }
    if (isArchiveName(entry.name)) {
      setSelected(entry);
      openExtractModal(entry);
      return;
    }
    await openPreview(entry);
  };

  const openPreview = async (entry: FileEntry) => {
    setPreviewPath(entry.path);
    setPreviewOpen(true);
    setPreviewContent('');
    setPreviewBinary(false);
    try {
      const data = await readFile(entry.path);
      if (data.is_binary) {
        setPreviewBinary(true);
        setPreviewContent('');
      } else {
        setPreviewContent(data.content ?? '');
      }
    } catch {
      setPreviewContent('无法读取文件');
    }
  };

  const savePreview = async () => {
    setPreviewSaving(true);
    try {
      await writeFile(previewPath, previewContent, false);
      setPreviewOpen(false);
      load(currentPath);
    } catch (e: unknown) {
      const msg = (e as { response?: { data?: { error?: string } } })?.response?.data?.error ?? '保存失败';
      setError(msg);
    } finally {
      setPreviewSaving(false);
    }
  };

  const handleMkdir = async () => {
    if (!mkdirName.trim()) return;
    const path = currentPath === '/' ? `/${mkdirName.trim()}` : `${currentPath}/${mkdirName.trim()}`;
    try {
      await mkdir(path);
      setMkdirOpen(false);
      setMkdirName('');
      load(currentPath);
    } catch (e: unknown) {
      const msg = (e as { response?: { data?: { error?: string } } })?.response?.data?.error ?? '创建失败';
      setError(msg);
    }
  };

  const handleRename = async () => {
    if (!selected || !renameName.trim()) return;
    const dir = selected.path.substring(0, selected.path.lastIndexOf('/'));
    const to = dir ? `${dir}/${renameName.trim()}` : `/${renameName.trim()}`;
    try {
      await renameFile(selected.path, to);
      setRenameOpen(false);
      setRenameName('');
      setSelected(null);
      load(currentPath);
    } catch (e: unknown) {
      const msg = (e as { response?: { data?: { error?: string } } })?.response?.data?.error ?? '重命名失败';
      setError(msg);
    }
  };

  const handleMoveCopy = async () => {
    if (!selected || !moveTarget.trim()) return;
    try {
      if (moveMode === 'move') {
        await moveFile(selected.path, moveTarget.trim());
      } else {
        await copyFile(selected.path, moveTarget.trim());
      }
      setMoveOpen(false);
      setMoveTarget('');
      setSelected(null);
      load(currentPath);
    } catch (e: unknown) {
      const msg = (e as { response?: { data?: { error?: string } } })?.response?.data?.error ?? '操作失败';
      setError(msg);
    }
  };

  const handleDelete = async () => {
    if (!selected) return;
    try {
      await deleteFile(selected.path, deleteRecursive);
      setDeleteOpen(false);
      setSelected(null);
      load(currentPath);
    } catch (e: unknown) {
      const msg = (e as { response?: { data?: { error?: string } } })?.response?.data?.error ?? '删除失败';
      setError(msg);
    }
  };

  const handleUpload = async (files: FileList | File[]) => {
    const list = Array.from(files);
    if (list.length === 0) return;
    setLoading(true);
    try {
      await uploadFiles(currentPath, list);
      load(currentPath);
    } catch (e: unknown) {
      const msg = (e as { response?: { data?: { error?: string } } })?.response?.data?.error ?? '上传失败';
      setError(msg);
    } finally {
      setLoading(false);
    }
  };

  const onDrop = (e: React.DragEvent) => {
    e.preventDefault();
    setDragOver(false);
    if (e.dataTransfer.files.length > 0) {
      handleUpload(e.dataTransfer.files);
    }
  };

  return (
    <Card
      className={`flex flex-col overflow-hidden ${fullPage ? 'flex-1 min-h-0 h-full' : ''}`}
      style={fullPage ? undefined : { height: '420px' }}
    >
      {/* Toolbar */}
      <div className="flex items-center gap-2 px-3 py-2 border-b border-default-200 flex-wrap shrink-0">
        <span className="text-xs font-semibold text-foreground/70">文件管理</span>
        {parent !== null && (
          <Button size="sm" variant="ghost" onPress={() => navigate(parent ?? '/')}>↑ 上级</Button>
        )}
        <Button size="sm" variant="ghost" onPress={() => load(currentPath)} isDisabled={loading}>刷新</Button>
        <Button size="sm" variant="ghost" onPress={() => { setMkdirName(''); setMkdirOpen(true); }}>新建文件夹</Button>
        <Button size="sm" variant="ghost" onPress={() => fileInputRef.current?.click()}>上传</Button>
        {(compressTargets.length > 0) && (
          <Button size="sm" variant="secondary" onPress={openCompressModal}>压缩</Button>
        )}
        <input
          ref={fileInputRef}
          type="file"
          multiple
          className="hidden"
          onChange={(e) => { if (e.target.files) handleUpload(e.target.files); e.target.value = ''; }}
        />
        <div className="flex items-center gap-1 ml-auto">
          <Switch size="sm" isSelected={showHidden} onChange={setShowHidden}>
            <span className="text-xs">隐藏文件</span>
          </Switch>
        </div>
      </div>

      {/* Quick paths */}
      <div className="flex items-center gap-1 px-3 py-1.5 border-b border-default-200 overflow-x-auto shrink-0">
        {QUICK_PATHS.map((p) => (
          <Chip
            key={p}
            size="sm"
            variant={currentPath === p ? 'primary' : 'soft'}
            className="cursor-pointer shrink-0"
            onClick={() => navigate(p)}
          >
            {p}
          </Chip>
        ))}
      </div>

      {/* Breadcrumb */}
      <div className="flex items-center gap-1 px-3 py-1.5 text-xs font-mono text-foreground/60 overflow-x-auto shrink-0">
        {breadcrumbs.map((c, i) => (
          <span key={c.path} className="flex items-center gap-1 shrink-0">
            {i > 0 && <span>/</span>}
            <button type="button" className="hover:text-foreground text-foreground/70" onClick={() => navigate(c.path)}>
              {c.label}
            </button>
          </span>
        ))}
      </div>

      {error && (
        <div className="px-3 py-1.5 text-xs text-danger bg-danger/10 shrink-0 flex items-center justify-between border-b border-danger/20">
          <span>{error}</span>
          <button type="button" className="opacity-60 hover:opacity-100" onClick={() => setError(null)}>×</button>
        </div>
      )}

      {/* File list */}
      <div
        className={`flex-1 min-h-0 overflow-auto ${dragOver ? 'ring-2 ring-inset ring-accent/40' : ''}`}
        onDragOver={(e) => { e.preventDefault(); setDragOver(true); }}
        onDragLeave={() => setDragOver(false)}
        onDrop={onDrop}
      >
        {loading && entries.length === 0 ? (
          <div className="flex items-center justify-center h-full text-sm text-foreground/50">加载中...</div>
        ) : (
          <table className="w-full text-xs">
            <thead className="sticky top-0 dm-table-head z-10">
              <tr className="text-foreground/50 border-b border-default-200">
                <th className="w-8 px-2 py-1.5" />
                <th className="text-left px-3 py-1.5 cursor-pointer" onClick={() => toggleSort('name')}>名称 {sortKey === 'name' ? (sortDir === 'asc' ? '↑' : '↓') : ''}</th>
                <th className="text-right px-2 py-1.5 cursor-pointer w-20" onClick={() => toggleSort('size')}>大小</th>
                <th className="text-left px-2 py-1.5 cursor-pointer w-24" onClick={() => toggleSort('mode')}>权限</th>
                <th className="text-left px-2 py-1.5 w-16">属主</th>
                <th className="text-right px-3 py-1.5 cursor-pointer w-36" onClick={() => toggleSort('modified')}>修改时间</th>
              </tr>
            </thead>
            <tbody>
              {filtered.map((entry) => (
                <tr
                  key={entry.path}
                  className={`border-b border-default-200 cursor-pointer dm-table-row ${
                    selected?.path === entry.path ? 'dm-table-row-selected' : ''
                  }`}
                  onClick={() => handleEntryClick(entry)}
                  onDoubleClick={() => handleEntryDoubleClick(entry)}
                >
                  <td className="px-2 py-1.5" onClick={(e) => e.stopPropagation()}>
                    <input
                      type="checkbox"
                      checked={checkedPaths.has(entry.path)}
                      onChange={() => toggleCheck(entry.path)}
                    />
                  </td>
                  <td className="px-3 py-1.5 font-mono truncate max-w-[200px]">
                    <span className="mr-1.5">{fileIcon(entry)}</span>
                    {entry.name}
                  </td>
                  <td className="text-right px-2 py-1.5 text-foreground/70">{entry.is_dir ? '-' : formatSize(entry.size)}</td>
                  <td className="px-2 py-1.5 font-mono text-foreground/60">{entry.mode}</td>
                  <td className="px-2 py-1.5 text-foreground/60">{entry.owner}</td>
                  <td className="text-right px-3 py-1.5 text-foreground/60">{formatTime(entry.modified)}</td>
                </tr>
              ))}
              {filtered.length === 0 && (
                <tr><td colSpan={6} className="text-center py-8 text-foreground/40">空目录</td></tr>
              )}
            </tbody>
          </table>
        )}
      </div>

      {/* Selection actions */}
      {selected && (
        <div className="flex items-center gap-2 px-3 py-1.5 border-t border-default-200 shrink-0 flex-wrap">
          <span className="text-xs text-foreground/50 truncate max-w-[180px]">{selected.name}</span>
          {!selected.is_dir && (
            <>
              <Button size="sm" variant="ghost" onPress={() => openPreview(selected)}>预览</Button>
              <Button size="sm" variant="ghost" onPress={() => window.open(downloadUrl(selected.path), '_blank')}>下载</Button>
              {isArchiveName(selected.name) && (
                <Button size="sm" variant="secondary" onPress={() => openExtractModal(selected)}>解压</Button>
              )}
            </>
          )}
          <Button size="sm" variant="ghost" onPress={() => { setRenameName(selected.name); setRenameOpen(true); }}>重命名</Button>
          <Button size="sm" variant="ghost" onPress={() => { setMoveMode('move'); setMoveTarget(''); setMoveOpen(true); }}>移动</Button>
          <Button size="sm" variant="ghost" onPress={() => { setMoveMode('copy'); setMoveTarget(''); setMoveOpen(true); }}>复制</Button>
          <Button size="sm" variant="danger" onPress={() => { setDeleteRecursive(selected.is_dir); setDeleteOpen(true); }}>删除</Button>
        </div>
      )}

      {/* Modals */}
      <SimpleModal open={mkdirOpen} title="新建文件夹" onClose={() => setMkdirOpen(false)} onConfirm={handleMkdir}>
        <input
          className="dm-input"
          placeholder="文件夹名称"
          value={mkdirName}
          onChange={(e) => setMkdirName(e.target.value)}
          onKeyDown={(e) => e.key === 'Enter' && handleMkdir()}
          autoFocus
        />
      </SimpleModal>

      <SimpleModal open={renameOpen} title="重命名" onClose={() => setRenameOpen(false)} onConfirm={handleRename}>
        <input
          className="dm-input"
          value={renameName}
          onChange={(e) => setRenameName(e.target.value)}
          onKeyDown={(e) => e.key === 'Enter' && handleRename()}
          autoFocus
        />
      </SimpleModal>

      <SimpleModal
        open={moveOpen}
        title={moveMode === 'move' ? '移动到' : '复制到'}
        onClose={() => setMoveOpen(false)}
        onConfirm={handleMoveCopy}
      >
        <input
          className="dm-input font-mono"
          placeholder="目标完整路径"
          value={moveTarget}
          onChange={(e) => setMoveTarget(e.target.value)}
          onKeyDown={(e) => e.key === 'Enter' && handleMoveCopy()}
          autoFocus
        />
      </SimpleModal>

      <SimpleModal open={deleteOpen} title="确认删除" onClose={() => setDeleteOpen(false)} onConfirm={handleDelete} confirmLabel="删除" danger>
        <p className="text-sm text-foreground/70">确定删除 <span className="font-mono text-danger">{selected?.path}</span>？</p>
        {selected?.is_dir && (
          <label className="flex items-center gap-2 mt-2 text-xs">
            <input type="checkbox" checked={deleteRecursive} onChange={(e) => setDeleteRecursive(e.target.checked)} />
            递归删除目录内容
          </label>
        )}
      </SimpleModal>

      <SimpleModal open={compressOpen} title="压缩文件" onClose={() => setCompressOpen(false)} onConfirm={handleCompress} confirmLabel="压缩">
        <p className="text-xs text-foreground/60 mb-2">已选 {compressTargets.length} 项</p>
        <div className="flex gap-2 mb-3">
          {(['zip', '7z', 'rar'] as ArchiveFormat[]).map((f) => (
            <button
              key={f}
              type="button"
              onClick={() => {
                setCompressFormat(f);
                setCompressOutput((prev) => prev.replace(/\.(zip|7z|rar)$/i, `.${f}`));
              }}
              className={`text-xs px-3 py-1.5 rounded-md uppercase transition-colors ${
                compressFormat === f
                  ? 'bg-accent/15 text-accent font-medium'
                  : 'text-foreground/50 hover:text-foreground/80 hover:bg-default-100'
              }`}
            >
              {f}
            </button>
          ))}
        </div>
        <input
          className="dm-input font-mono"
          placeholder="输出路径，如 /tmp/archive.zip"
          value={compressOutput}
          onChange={(e) => setCompressOutput(e.target.value)}
          onKeyDown={(e) => e.key === 'Enter' && handleCompress()}
          autoFocus
        />
        {compressFormat === 'rar' && (
          <p className="text-xs text-foreground/50 mt-2">RAR 压缩需要设备上安装 rar 命令</p>
        )}
      </SimpleModal>

      <SimpleModal open={extractOpen} title="解压文件" onClose={() => setExtractOpen(false)} onConfirm={handleExtract} confirmLabel="解压">
        <p className="text-xs text-foreground/60 mb-2 font-mono truncate">{selected?.path}</p>
        <input
          className="dm-input font-mono mb-2"
          placeholder="解压目标目录"
          value={extractDest}
          onChange={(e) => setExtractDest(e.target.value)}
          onKeyDown={(e) => e.key === 'Enter' && handleExtract()}
          autoFocus
        />
        <label className="flex items-center gap-2 text-xs">
          <input type="checkbox" checked={extractOverwrite} onChange={(e) => setExtractOverwrite(e.target.checked)} />
          覆盖已存在文件
        </label>
        {selected && isArchiveName(selected.name) && selected.name.endsWith('.rar') && (
          <p className="text-xs text-foreground/50 mt-2">RAR 解压需要设备上安装 unrar 或 7z 命令</p>
        )}
      </SimpleModal>

      {/* Preview drawer */}
      {previewOpen && (
        <div className="fixed inset-0 z-50 flex flex-col bg-black/40" onClick={() => setPreviewOpen(false)}>
          <div className="mt-auto" onClick={(e) => e.stopPropagation()}>
            <Card
              className="rounded-t-xl rounded-b-none border border-default-200 flex flex-col"
              style={{ height: '70dvh' }}
            >
            <div className="flex items-center gap-2 px-4 py-3 border-b border-default-200 shrink-0">
              <span className="text-sm font-mono truncate flex-1">{previewPath}</span>
              {!previewBinary && (
                <Button size="sm" variant="secondary" onPress={savePreview} isDisabled={previewSaving}>
                  {previewSaving ? '保存中...' : '保存'}
                </Button>
              )}
              {!previewBinary && (
                <Button size="sm" variant="ghost" onPress={() => window.open(downloadUrl(previewPath), '_blank')}>下载</Button>
              )}
              <Button size="sm" variant="ghost" onPress={() => setPreviewOpen(false)}>关闭</Button>
            </div>
            <div className="flex-1 min-h-0 p-4">
              {previewBinary ? (
                <div className="flex flex-col items-center justify-center h-full gap-3 text-foreground/60">
                  <p className="text-sm">二进制文件，无法预览</p>
                  <Button size="sm" variant="secondary" onPress={() => window.open(downloadUrl(previewPath), '_blank')}>下载文件</Button>
                </div>
              ) : (
                <textarea
                  className="dm-input h-full font-mono text-xs resize-none"
                  value={previewContent}
                  onChange={(e) => setPreviewContent(e.target.value)}
                  spellCheck={false}
                />
              )}
            </div>
            </Card>
          </div>
        </div>
      )}
    </Card>
  );
}

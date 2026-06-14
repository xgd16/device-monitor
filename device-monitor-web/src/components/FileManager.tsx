import { useState, useEffect, useCallback, useMemo, useRef, type MouseEvent } from 'react';
import { Card, Button, Switch } from '@heroui/react';
import {
  RiFolderLine,
  RiFolderFill,
  RiFileLine,
  RiLinksLine,
  RiFileZipLine,
  RiImageLine,
  RiMovieLine,
  RiFileCodeLine,
  RiFilePdfLine,
  RiMarkdownLine,
  RiRefreshLine,
  RiFolderAddLine,
  RiUpload2Line,
  RiArrowUpLine,
  RiFileCopyLine,
  RiScissorsCutLine,
  RiDeleteBinLine,
  RiInboxUnarchiveLine,
  RiClipboardLine,
  RiCloseLine,
  RiEyeLine,
} from '@remixicon/react';
import type { FileEntry } from '../types';
import {
  listFiles, uploadFiles,
  mkdir, moveFile, copyFile, deleteFile,
  compressFiles, extractFiles, type ArchiveFormat,
} from '../api';
import { PathAddressBar } from './PathAddressBar';
import { FilePreviewModal } from './FilePreviewModal';
import { isPreviewable } from '../utils/filePreview';

type SortKey = 'name' | 'size' | 'modified' | 'mode';
type SortDir = 'asc' | 'desc';

type ClipboardItem = { mode: 'copy' | 'cut'; entry: FileEntry };

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

function isArchiveName(name: string): boolean {
  const ext = name.split('.').pop()?.toLowerCase();
  return ext === 'zip' || ext === '7z' || ext === 'rar';
}

function FileTypeIcon({ entry, className = 'w-4 h-4 shrink-0' }: { entry: FileEntry; className?: string }) {
  const props = { className, 'aria-hidden': true as const };
  if (entry.is_symlink) return <RiLinksLine {...props} />;
  if (entry.is_dir) return <RiFolderFill {...props} className={`${className} text-amber-500/80`} />;
  const ext = entry.name.split('.').pop()?.toLowerCase();
  if (['zip', '7z', 'rar'].includes(ext ?? '')) return <RiFileZipLine {...props} className={`${className} text-violet-500/80`} />;
  if (['jpg', 'jpeg', 'png', 'gif', 'webp', 'svg', 'bmp'].includes(ext ?? '')) return <RiImageLine {...props} />;
  if (['mp4', 'mkv', 'avi', 'webm', 'mov'].includes(ext ?? '')) return <RiMovieLine {...props} />;
  if (ext === 'pdf') return <RiFilePdfLine {...props} className={`${className} text-red-500/70`} />;
  if (['md', 'markdown'].includes(ext ?? '')) return <RiMarkdownLine {...props} className={`${className} text-sky-500/70`} />;
  if (['sh', 'py', 'rs', 'js', 'ts', 'json', 'yaml', 'toml'].includes(ext ?? '')) return <RiFileCodeLine {...props} />;
  return <RiFileLine {...props} />;
}

function IconBtn({
  label,
  onClick,
  danger,
  children,
}: {
  label: string;
  onClick: () => void;
  danger?: boolean;
  children: React.ReactNode;
}) {
  return (
    <button
      type="button"
      title={label}
      aria-label={label}
      onClick={(e) => { e.stopPropagation(); onClick(); }}
      className={`p-1.5 rounded-md transition-colors ${
        danger
          ? 'text-danger hover:bg-danger/10'
          : 'text-foreground/60 hover:text-foreground hover:bg-default-100'
      }`}
    >
      {children}
    </button>
  );
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
  const [checkedPaths, setCheckedPaths] = useState<Set<string>>(new Set());
  const [clipboard, setClipboard] = useState<ClipboardItem | null>(null);

  const [mkdirOpen, setMkdirOpen] = useState(false);
  const [mkdirName, setMkdirName] = useState('');
  const [deleteOpen, setDeleteOpen] = useState(false);
  const [deleteTarget, setDeleteTarget] = useState<FileEntry | null>(null);
  const [deleteRecursive, setDeleteRecursive] = useState(false);
  const [compressOpen, setCompressOpen] = useState(false);
  const [compressPaths, setCompressPaths] = useState<string[]>([]);
  const [compressFormat, setCompressFormat] = useState<ArchiveFormat>('zip');
  const [compressOutput, setCompressOutput] = useState('');
  const [extractOpen, setExtractOpen] = useState(false);
  const [extractTarget, setExtractTarget] = useState<FileEntry | null>(null);
  const [extractDest, setExtractDest] = useState('');
  const [extractOverwrite, setExtractOverwrite] = useState(false);

  const [previewOpen, setPreviewOpen] = useState(false);
  const [previewPath, setPreviewPath] = useState('');
  const [previewName, setPreviewName] = useState('');

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
    if (!showHidden) list = list.filter((e) => !e.name.startsWith('.'));
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
    if (sortKey === key) setSortDir((d) => (d === 'asc' ? 'desc' : 'asc'));
    else {
      setSortKey(key);
      setSortDir(key === 'name' ? 'asc' : 'desc');
    }
  };

  const joinPath = (dir: string, name: string) => (dir === '/' ? `/${name}` : `${dir}/${name}`);

  const handleEntryClick = (entry: FileEntry, e: MouseEvent) => {
    if ((e.target as HTMLElement).closest('input, button')) return;
    if (entry.is_dir) navigate(entry.path);
  };

  const handleEntryDoubleClick = (entry: FileEntry) => {
    if (entry.is_dir) return;
    if (isArchiveName(entry.name)) {
      openExtractModalFor(entry);
      return;
    }
    if (isPreviewable(entry.name)) {
      openPreview(entry);
    }
  };

  const openPreview = (entry: FileEntry) => {
    if (entry.is_dir) return;
    setPreviewPath(entry.path);
    setPreviewName(entry.name);
    setPreviewOpen(true);
  };

  const handleCopyEntry = (entry: FileEntry) => {
    setClipboard({ mode: 'copy', entry });
  };

  const handleCutEntry = (entry: FileEntry) => {
    setClipboard({ mode: 'cut', entry });
  };

  const handlePaste = async () => {
    if (!clipboard) return;
    const { entry, mode } = clipboard;
    if (entry.path === joinPath(currentPath, entry.name) && mode === 'cut') {
      setClipboard(null);
      return;
    }
    let dest = joinPath(currentPath, entry.name);
    if (mode === 'copy' && entry.path === dest) {
      const dot = entry.name.lastIndexOf('.');
      dest = dot > 0
        ? joinPath(currentPath, `${entry.name.slice(0, dot)}_copy${entry.name.slice(dot)}`)
        : joinPath(currentPath, `${entry.name}_copy`);
    }
    setLoading(true);
    setError(null);
    try {
      if (mode === 'cut') {
        await moveFile(entry.path, dest);
      } else {
        await copyFile(entry.path, dest);
      }
      setClipboard(null);
      load(currentPath);
    } catch (e: unknown) {
      const msg = (e as { response?: { data?: { error?: string } } })?.response?.data?.error ?? '粘贴失败';
      setError(msg);
    } finally {
      setLoading(false);
    }
  };

  const openDeleteFor = (entry: FileEntry) => {
    setDeleteTarget(entry);
    setDeleteRecursive(entry.is_dir);
    setDeleteOpen(true);
  };

  const handleDelete = async () => {
    if (!deleteTarget) return;
    try {
      await deleteFile(deleteTarget.path, deleteRecursive);
      if (clipboard?.entry.path === deleteTarget.path) setClipboard(null);
      setDeleteOpen(false);
      setDeleteTarget(null);
      load(currentPath);
    } catch (e: unknown) {
      const msg = (e as { response?: { data?: { error?: string } } })?.response?.data?.error ?? '删除失败';
      setError(msg);
    }
  };

  const openCompressFor = (entry: FileEntry) => {
    const base = currentPath === '/' ? '/archive' : `${currentPath}/archive`;
    setCompressPaths([entry.path]);
    setCompressFormat('zip');
    setCompressOutput(`${base}.zip`);
    setCompressOpen(true);
  };

  const openCompressBulk = () => {
    const paths = entries.filter((e) => checkedPaths.has(e.path)).map((e) => e.path);
    if (paths.length === 0) return;
    const base = currentPath === '/' ? '/archive' : `${currentPath}/archive`;
    setCompressPaths(paths);
    setCompressFormat('zip');
    setCompressOutput(`${base}.zip`);
    setCompressOpen(true);
  };

  const handleCompress = async () => {
    if (compressPaths.length === 0 || !compressOutput.trim()) return;
    setLoading(true);
    setError(null);
    try {
      await compressFiles(compressPaths, compressOutput.trim(), compressFormat);
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

  const openExtractModalFor = (entry: FileEntry) => {
    const base = entry.name.replace(/\.(zip|7z|rar)$/i, '');
    setExtractTarget(entry);
    setExtractDest(currentPath === '/' ? `/${base}` : `${currentPath}/${base}`);
    setExtractOverwrite(false);
    setExtractOpen(true);
  };

  const handleExtract = async () => {
    if (!extractTarget || !extractDest.trim()) return;
    setLoading(true);
    setError(null);
    try {
      await extractFiles(extractTarget.path, extractDest.trim(), extractOverwrite);
      setExtractOpen(false);
      setExtractTarget(null);
      load(currentPath);
    } catch (e: unknown) {
      const msg = (e as { response?: { data?: { error?: string } } })?.response?.data?.error ?? '解压失败';
      setError(msg);
    } finally {
      setLoading(false);
    }
  };

  const handleMkdir = async () => {
    if (!mkdirName.trim()) return;
    try {
      await mkdir(joinPath(currentPath, mkdirName.trim()));
      setMkdirOpen(false);
      setMkdirName('');
      load(currentPath);
    } catch (e: unknown) {
      const msg = (e as { response?: { data?: { error?: string } } })?.response?.data?.error ?? '创建失败';
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

  return (
    <Card className={`flex flex-col overflow-hidden ${fullPage ? 'flex-1 min-h-0 h-full' : ''}`} style={fullPage ? undefined : { height: '420px' }}>
      {/* Toolbar */}
      <div className="flex items-center gap-1 px-3 py-2 border-b border-default-200 flex-wrap shrink-0">
        <RiFolderLine className="w-4 h-4 text-foreground/50 mr-1" aria-hidden />
        <span className="text-xs font-semibold text-foreground/70 mr-2">文件管理</span>
        {parent !== null && (
          <Button size="sm" variant="ghost" onPress={() => navigate(parent ?? '/')}>
            <RiArrowUpLine className="w-3.5 h-3.5" /> 上级
          </Button>
        )}
        <Button size="sm" variant="ghost" onPress={() => load(currentPath)} isDisabled={loading}>
          <RiRefreshLine className="w-3.5 h-3.5" /> 刷新
        </Button>
        <Button size="sm" variant="ghost" onPress={() => { setMkdirName(''); setMkdirOpen(true); }}>
          <RiFolderAddLine className="w-3.5 h-3.5" /> 新建
        </Button>
        <Button size="sm" variant="ghost" onPress={() => fileInputRef.current?.click()}>
          <RiUpload2Line className="w-3.5 h-3.5" /> 上传
        </Button>
        {clipboard && (
          <Button size="sm" variant="secondary" onPress={handlePaste}>
            <RiClipboardLine className="w-3.5 h-3.5" />
            粘贴{clipboard.mode === 'cut' ? '(剪切)' : '(复制)'}
          </Button>
        )}
        {checkedPaths.size > 0 && (
          <Button size="sm" variant="secondary" onPress={openCompressBulk}>
            <RiFileZipLine className="w-3.5 h-3.5" /> 压缩 ({checkedPaths.size})
          </Button>
        )}
        <input ref={fileInputRef} type="file" multiple className="hidden"
          onChange={(e) => { if (e.target.files) handleUpload(e.target.files); e.target.value = ''; }} />
        <div className="flex items-center gap-1 ml-auto">
          <Switch size="sm" isSelected={showHidden} onChange={setShowHidden}>
            <span className="text-xs">隐藏文件</span>
          </Switch>
        </div>
      </div>

      {/* Clipboard banner */}
      {clipboard && (
        <div className="px-3 py-1.5 text-xs bg-accent/10 text-accent border-b border-accent/20 flex items-center gap-2 shrink-0">
          <RiScissorsCutLine className="w-3.5 h-3.5 shrink-0" />
          <span className="truncate flex-1">
            已{clipboard.mode === 'cut' ? '剪切' : '复制'}：<span className="font-mono">{clipboard.entry.name}</span>
            — 进入目标目录后点击「粘贴」
          </span>
          <button type="button" onClick={() => setClipboard(null)} className="p-0.5 hover:opacity-70">
            <RiCloseLine className="w-4 h-4" />
          </button>
        </div>
      )}

      {/* Path address bar */}
      <PathAddressBar path={currentPath} onNavigate={navigate} breadcrumbs={breadcrumbs} />

      {error && (
        <div className="px-3 py-1.5 text-xs text-danger bg-danger/10 shrink-0 flex items-center justify-between border-b border-danger/20">
          <span>{error}</span>
          <button type="button" onClick={() => setError(null)}><RiCloseLine className="w-4 h-4" /></button>
        </div>
      )}

      {/* File list */}
      <div
        className={`flex-1 min-h-0 overflow-auto ${dragOver ? 'ring-2 ring-inset ring-accent/40' : ''}`}
        onDragOver={(e) => { e.preventDefault(); setDragOver(true); }}
        onDragLeave={() => setDragOver(false)}
        onDrop={(e) => { e.preventDefault(); setDragOver(false); if (e.dataTransfer.files.length) handleUpload(e.dataTransfer.files); }}
      >
        {loading && entries.length === 0 ? (
          <div className="flex items-center justify-center h-full text-sm text-foreground/50">加载中...</div>
        ) : (
          <table className="w-full text-xs">
            <thead className="sticky top-0 dm-table-head z-10">
              <tr className="text-foreground/50 border-b border-default-200">
                <th className="w-8 px-2 py-2" />
                <th className="text-left px-3 py-2 cursor-pointer" onClick={() => toggleSort('name')}>
                  名称 {sortKey === 'name' ? (sortDir === 'asc' ? '↑' : '↓') : ''}
                </th>
                <th className="text-right px-2 py-2 cursor-pointer w-20 hidden sm:table-cell" onClick={() => toggleSort('size')}>大小</th>
                <th className="text-left px-2 py-2 cursor-pointer w-24 hidden md:table-cell" onClick={() => toggleSort('mode')}>权限</th>
                <th className="text-left px-2 py-2 w-12 hidden lg:table-cell">属主</th>
                <th className="text-right px-2 py-2 cursor-pointer w-32 hidden sm:table-cell" onClick={() => toggleSort('modified')}>修改时间</th>
                <th className="text-right px-2 py-2 w-44 sticky right-0 dm-table-head">操作</th>
              </tr>
            </thead>
            <tbody>
              {filtered.map((entry) => (
                <tr
                  key={entry.path}
                  className={`border-b border-default-200 dm-table-row ${
                    entry.is_dir ? 'cursor-pointer' : ''
                  } ${
                    clipboard?.entry.path === entry.path ? 'dm-table-row-selected' : ''
                  } ${checkedPaths.has(entry.path) ? 'bg-accent/5' : ''}`}
                  onClick={(e) => handleEntryClick(entry, e)}
                  onDoubleClick={() => handleEntryDoubleClick(entry)}
                >
                  <td className="px-2 py-1.5">
                    <input type="checkbox" checked={checkedPaths.has(entry.path)} onChange={() => toggleCheck(entry.path)} />
                  </td>
                  <td className="px-3 py-1.5 font-mono truncate max-w-[160px] sm:max-w-none">
                    <span className="inline-flex items-center gap-1.5">
                      <FileTypeIcon entry={entry} />
                      <span className="truncate">{entry.name}</span>
                    </span>
                  </td>
                  <td className="text-right px-2 py-1.5 text-foreground/70 hidden sm:table-cell">
                    {entry.is_dir ? '-' : formatSize(entry.size)}
                  </td>
                  <td className="px-2 py-1.5 font-mono text-foreground/60 hidden md:table-cell">{entry.mode}</td>
                  <td className="px-2 py-1.5 text-foreground/60 hidden lg:table-cell">{entry.owner}</td>
                  <td className="text-right px-2 py-1.5 text-foreground/60 hidden sm:table-cell">{formatTime(entry.modified)}</td>
                  <td className="px-1 py-1 sticky right-0 dm-table-head">
                    <div className="flex items-center justify-end gap-0.5">
                      {!entry.is_dir && isPreviewable(entry.name) && (
                        <IconBtn label="预览" onClick={() => openPreview(entry)}>
                          <RiEyeLine className="w-4 h-4" />
                        </IconBtn>
                      )}
                      <IconBtn label="复制" onClick={() => handleCopyEntry(entry)}>
                        <RiFileCopyLine className="w-4 h-4" />
                      </IconBtn>
                      <IconBtn label="剪切" onClick={() => handleCutEntry(entry)}>
                        <RiScissorsCutLine className="w-4 h-4" />
                      </IconBtn>
                      {!entry.is_dir && isArchiveName(entry.name) ? (
                        <IconBtn label="解压" onClick={() => openExtractModalFor(entry)}>
                          <RiInboxUnarchiveLine className="w-4 h-4" />
                        </IconBtn>
                      ) : (
                        <IconBtn label="压缩" onClick={() => openCompressFor(entry)}>
                          <RiFileZipLine className="w-4 h-4" />
                        </IconBtn>
                      )}
                      <IconBtn label="删除" onClick={() => openDeleteFor(entry)} danger>
                        <RiDeleteBinLine className="w-4 h-4" />
                      </IconBtn>
                    </div>
                  </td>
                </tr>
              ))}
              {filtered.length === 0 && (
                <tr><td colSpan={7} className="text-center py-8 text-foreground/40">空目录</td></tr>
              )}
            </tbody>
          </table>
        )}
      </div>

      {/* Modals */}
      <SimpleModal open={mkdirOpen} title="新建文件夹" onClose={() => setMkdirOpen(false)} onConfirm={handleMkdir}>
        <input className="dm-input" placeholder="文件夹名称" value={mkdirName}
          onChange={(e) => setMkdirName(e.target.value)} onKeyDown={(e) => e.key === 'Enter' && handleMkdir()} autoFocus />
      </SimpleModal>

      <SimpleModal open={deleteOpen} title="确认删除" onClose={() => setDeleteOpen(false)} onConfirm={handleDelete} confirmLabel="删除" danger>
        <p className="text-sm text-foreground/70">确定删除 <span className="font-mono text-danger">{deleteTarget?.path}</span>？</p>
        {deleteTarget?.is_dir && (
          <label className="flex items-center gap-2 mt-2 text-xs">
            <input type="checkbox" checked={deleteRecursive} onChange={(e) => setDeleteRecursive(e.target.checked)} />
            递归删除目录内容
          </label>
        )}
      </SimpleModal>

      <SimpleModal open={compressOpen} title="压缩文件" onClose={() => setCompressOpen(false)} onConfirm={handleCompress} confirmLabel="压缩">
        <p className="text-xs text-foreground/60 mb-2">已选 {compressPaths.length} 项</p>
        <div className="flex gap-2 mb-3">
          {(['zip', '7z', 'rar'] as ArchiveFormat[]).map((f) => (
            <button key={f} type="button"
              onClick={() => { setCompressFormat(f); setCompressOutput((p) => p.replace(/\.(zip|7z|rar)$/i, `.${f}`)); }}
              className={`text-xs px-3 py-1.5 rounded-md uppercase transition-colors ${
                compressFormat === f ? 'bg-accent/15 text-accent font-medium' : 'text-foreground/50 hover:bg-default-100'
              }`}>{f}</button>
          ))}
        </div>
        <input className="dm-input font-mono" placeholder="输出路径" value={compressOutput}
          onChange={(e) => setCompressOutput(e.target.value)} onKeyDown={(e) => e.key === 'Enter' && handleCompress()} autoFocus />
        {compressFormat === 'rar' && <p className="text-xs text-foreground/50 mt-2">RAR 压缩需要设备上安装 rar 命令</p>}
      </SimpleModal>

      <SimpleModal open={extractOpen} title="解压文件" onClose={() => setExtractOpen(false)} onConfirm={handleExtract} confirmLabel="解压">
        <p className="text-xs text-foreground/60 mb-2 font-mono truncate">{extractTarget?.path}</p>
        <input className="dm-input font-mono mb-2" placeholder="解压目标目录" value={extractDest}
          onChange={(e) => setExtractDest(e.target.value)} onKeyDown={(e) => e.key === 'Enter' && handleExtract()} autoFocus />
        <label className="flex items-center gap-2 text-xs">
          <input type="checkbox" checked={extractOverwrite} onChange={(e) => setExtractOverwrite(e.target.checked)} />
          覆盖已存在文件
        </label>
      </SimpleModal>

      <FilePreviewModal
        open={previewOpen}
        path={previewPath}
        name={previewName}
        onClose={() => setPreviewOpen(false)}
        onSaved={() => load(currentPath)}
      />
    </Card>
  );
}

import { useState, useEffect } from 'react';
import { Card, Button } from '@heroui/react';
import ReactMarkdown from 'react-markdown';
import remarkGfm from 'remark-gfm';
import {
  RiFileLine,
  RiImageLine,
  RiMovieLine,
  RiFilePdfLine,
  RiMarkdownLine,
  RiCodeLine,
  RiEyeLine,
  RiEditLine,
} from '@remixicon/react';
import { readFile, writeFile, previewUrl, downloadUrl } from '../api';
import { getPreviewKind, type PreviewKind } from '../utils/filePreview';

interface FilePreviewModalProps {
  open: boolean;
  path: string;
  name: string;
  onClose: () => void;
  onSaved?: () => void;
}

function KindIcon({ kind }: { kind: PreviewKind }) {
  const cls = 'w-4 h-4 shrink-0 text-foreground/50';
  switch (kind) {
    case 'image': return <RiImageLine className={cls} />;
    case 'video': return <RiMovieLine className={cls} />;
    case 'pdf': return <RiFilePdfLine className={cls} />;
    case 'markdown': return <RiMarkdownLine className={cls} />;
    case 'text': return <RiCodeLine className={cls} />;
    default: return <RiFileLine className={cls} />;
  }
}

export function FilePreviewModal({ open, path, name, onClose, onSaved }: FilePreviewModalProps) {
  const kind = getPreviewKind(name);
  const [loading, setLoading] = useState(false);
  const [content, setContent] = useState('');
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [mdView, setMdView] = useState<'preview' | 'edit'>('preview');

  const needsTextLoad = kind === 'text' || kind === 'markdown';
  const mediaUrl = previewUrl(path);

  useEffect(() => {
    if (!open) return;
    setError(null);
    setContent('');
    setMdView('preview');
    if (!needsTextLoad) return;

    let cancelled = false;
    setLoading(true);
    readFile(path)
      .then((data) => {
        if (cancelled) return;
        if (data.is_binary) {
          setError('无法以文本读取该文件');
        } else {
          setContent(data.content ?? '');
        }
      })
      .catch(() => {
        if (!cancelled) setError('读取文件失败');
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });

    return () => { cancelled = true; };
  }, [open, path, needsTextLoad]);

  const handleSave = async () => {
    setSaving(true);
    try {
      await writeFile(path, content, false);
      onSaved?.();
      onClose();
    } catch {
      setError('保存失败');
    } finally {
      setSaving(false);
    }
  };

  if (!open) return null;

  const canEdit = kind === 'text' || kind === 'markdown';
  const showSave = canEdit && (kind !== 'markdown' || mdView === 'edit');

  return (
    <div className="fixed inset-0 z-50 flex flex-col bg-black/40" onClick={onClose}>
      <div className="mt-auto" onClick={(e) => e.stopPropagation()}>
        <Card className="rounded-t-xl rounded-b-none border border-default-200 flex flex-col" style={{ height: '75dvh' }}>
          <div className="flex items-center gap-2 px-4 py-3 border-b border-default-200 shrink-0 flex-wrap">
            <KindIcon kind={kind} />
            <span className="text-sm font-mono truncate flex-1 min-w-0">{path}</span>
            {kind === 'markdown' && (
              <div className="flex rounded-md overflow-hidden border border-default-200 text-xs shrink-0">
                <button
                  type="button"
                  className={`px-2.5 py-1 flex items-center gap-1 ${mdView === 'preview' ? 'bg-accent/15 text-accent' : 'hover:bg-default-100'}`}
                  onClick={() => setMdView('preview')}
                >
                  <RiEyeLine className="w-3.5 h-3.5" /> 预览
                </button>
                <button
                  type="button"
                  className={`px-2.5 py-1 flex items-center gap-1 border-l border-default-200 ${mdView === 'edit' ? 'bg-accent/15 text-accent' : 'hover:bg-default-100'}`}
                  onClick={() => setMdView('edit')}
                >
                  <RiEditLine className="w-3.5 h-3.5" /> 编辑
                </button>
              </div>
            )}
            {showSave && (
              <Button size="sm" variant="secondary" onPress={handleSave} isDisabled={saving || loading}>
                {saving ? '保存中...' : '保存'}
              </Button>
            )}
            <Button size="sm" variant="ghost" onPress={() => window.open(downloadUrl(path), '_blank')}>下载</Button>
            <Button size="sm" variant="ghost" onPress={onClose}>关闭</Button>
          </div>

          <div className="flex-1 min-h-0 overflow-auto p-4">
            {loading && (
              <div className="flex items-center justify-center h-full text-sm text-foreground/50">加载中...</div>
            )}
            {!loading && error && (
              <div className="flex flex-col items-center justify-center h-full gap-3 text-foreground/60">
                <RiFileLine className="w-10 h-10" />
                <p className="text-sm">{error}</p>
                <Button size="sm" variant="secondary" onPress={() => window.open(downloadUrl(path), '_blank')}>下载文件</Button>
              </div>
            )}
            {!loading && !error && kind === 'image' && (
              <div className="flex items-center justify-center h-full min-h-[200px]">
                <img
                  src={mediaUrl}
                  alt={name}
                  className="max-w-full max-h-full object-contain rounded-md"
                />
              </div>
            )}
            {!loading && !error && kind === 'video' && (
              <div className="flex items-center justify-center h-full">
                <video
                  src={mediaUrl}
                  controls
                  className="max-w-full max-h-full rounded-md bg-black"
                  playsInline
                >
                  您的浏览器不支持视频播放
                </video>
              </div>
            )}
            {!loading && !error && kind === 'pdf' && (
              <iframe
                src={mediaUrl}
                title={name}
                className="w-full h-full min-h-[400px] rounded-md border border-default-200 bg-white"
              />
            )}
            {!loading && !error && kind === 'markdown' && mdView === 'preview' && (
              <article className="dm-markdown max-w-3xl mx-auto">
                <ReactMarkdown remarkPlugins={[remarkGfm]}>{content}</ReactMarkdown>
              </article>
            )}
            {!loading && !error && kind === 'markdown' && mdView === 'edit' && (
              <textarea
                className="dm-input h-full min-h-[300px] font-mono text-xs resize-none"
                value={content}
                onChange={(e) => setContent(e.target.value)}
                spellCheck={false}
              />
            )}
            {!loading && !error && kind === 'text' && (
              <textarea
                className="dm-input h-full min-h-[300px] font-mono text-xs resize-none"
                value={content}
                onChange={(e) => setContent(e.target.value)}
                spellCheck={false}
              />
            )}
            {!loading && !error && kind === 'binary' && (
              <div className="flex flex-col items-center justify-center h-full gap-3 text-foreground/60">
                <RiFileLine className="w-10 h-10" />
                <p className="text-sm">该文件类型暂不支持预览</p>
                <Button size="sm" variant="secondary" onPress={() => window.open(downloadUrl(path), '_blank')}>下载文件</Button>
              </div>
            )}
          </div>
        </Card>
      </div>
    </div>
  );
}

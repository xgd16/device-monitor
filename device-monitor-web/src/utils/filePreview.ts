export type PreviewKind = 'image' | 'video' | 'pdf' | 'markdown' | 'text' | 'binary';

const IMAGE_EXTS = new Set(['jpg', 'jpeg', 'png', 'gif', 'webp', 'svg', 'bmp', 'ico']);
const VIDEO_EXTS = new Set(['mp4', 'webm', 'mkv', 'mov', 'avi', 'm4v']);
const MARKDOWN_EXTS = new Set(['md', 'markdown', 'mdx']);

export function fileExtension(name: string): string {
  const dot = name.lastIndexOf('.');
  if (dot <= 0) return '';
  return name.slice(dot + 1).toLowerCase();
}

export function getPreviewKind(name: string, isBinary = false): PreviewKind {
  const ext = fileExtension(name);
  if (IMAGE_EXTS.has(ext)) return 'image';
  if (VIDEO_EXTS.has(ext)) return 'video';
  if (ext === 'pdf') return 'pdf';
  if (MARKDOWN_EXTS.has(ext)) return 'markdown';
  if (isBinary) return 'binary';
  return 'text';
}

export function isPreviewable(name: string): boolean {
  const kind = getPreviewKind(name);
  return kind !== 'binary';
}

import { useState, useEffect, useRef } from 'react';
import { RiFolderLine, RiArrowRightLine } from '@remixicon/react';

interface PathAddressBarProps {
  path: string;
  onNavigate: (path: string) => void;
  breadcrumbs: { label: string; path: string }[];
}

function normalizePath(input: string): string {
  let p = input.trim();
  if (!p) return '/';
  if (!p.startsWith('/')) p = `/${p}`;
  // 合并连续斜杠
  p = p.replace(/\/+/g, '/');
  if (p.length > 1 && p.endsWith('/')) p = p.slice(0, -1);
  return p;
}

export function PathAddressBar({ path, onNavigate, breadcrumbs }: PathAddressBarProps) {
  const [draft, setDraft] = useState(path);
  const [editing, setEditing] = useState(false);
  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (!editing) setDraft(path);
  }, [path, editing]);

  const submit = () => {
    setEditing(false);
    const next = normalizePath(draft);
    if (next !== path) onNavigate(next);
  };

  const startEdit = () => {
    setDraft(path);
    setEditing(true);
    requestAnimationFrame(() => inputRef.current?.select());
  };

  return (
    <div className="shrink-0 border-b border-default-200">
      <div className="flex items-center gap-2 px-3 py-1.5">
        <RiFolderLine className="w-3.5 h-3.5 shrink-0 text-foreground/50" aria-hidden />
        <input
          ref={inputRef}
          type="text"
          className="dm-input flex-1 min-w-0 py-1 px-2 text-xs font-mono"
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          onFocus={() => setEditing(true)}
          onKeyDown={(e) => {
            if (e.key === 'Enter') submit();
            if (e.key === 'Escape') {
              setDraft(path);
              setEditing(false);
              inputRef.current?.blur();
            }
          }}
          onBlur={() => {
            if (editing) submit();
          }}
          spellCheck={false}
          aria-label="当前路径"
        />
        <button
          type="button"
          title="前往"
          aria-label="前往"
          onMouseDown={(e) => e.preventDefault()}
          onClick={submit}
          className="shrink-0 p-1.5 rounded-md text-foreground/60 hover:text-foreground hover:bg-default-100 transition-colors"
        >
          <RiArrowRightLine className="w-4 h-4" />
        </button>
      </div>
      <div className="flex items-center gap-1 px-3 pb-1.5 text-xs font-mono text-foreground/50 overflow-x-auto">
        {breadcrumbs.map((c, i) => (
          <span key={c.path} className="flex items-center gap-1 shrink-0">
            {i > 0 && <span className="text-foreground/30">/</span>}
            <button
              type="button"
              className="hover:text-foreground text-foreground/60"
              onMouseDown={(e) => e.preventDefault()}
              onClick={() => onNavigate(c.path)}
            >
              {c.label}
            </button>
          </span>
        ))}
        {!editing && (
          <button type="button" className="ml-1 text-accent/80 hover:text-accent text-[10px]" onClick={startEdit}>
            编辑
          </button>
        )}
      </div>
    </div>
  );
}

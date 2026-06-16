import { useState, useCallback, type ReactNode } from 'react';
import ReactMarkdown from 'react-markdown';
import remarkGfm from 'remark-gfm';
import { RiFileCopyLine, RiCheckLine } from '@remixicon/react';
import { highlightCode, guessLangFromFilename, normalizeHighlightLang } from '../utils/highlightCode';

function CopyButton({ text }: { text: string }) {
  const [copied, setCopied] = useState(false);

  const handleCopy = useCallback(async () => {
    try {
      await navigator.clipboard.writeText(text);
      setCopied(true);
      window.setTimeout(() => setCopied(false), 2000);
    } catch {
      // ignore
    }
  }, [text]);

  return (
    <button
      type="button"
      onClick={handleCopy}
      className="inline-flex items-center gap-1 px-2 py-0.5 rounded text-[10px] font-mono opacity-60 hover:opacity-100 hover:bg-default-200/80 transition-colors"
    >
      {copied ? (
        <>
          <RiCheckLine className="w-3 h-3 text-success" /> 已复制
        </>
      ) : (
        <>
          <RiFileCopyLine className="w-3 h-3" /> 复制
        </>
      )}
    </button>
  );
}

function MarkdownCodeBlock({
  className,
  children,
}: {
  className?: string;
  children?: ReactNode;
}) {
  const text = String(children ?? '').replace(/\n$/, '');
  const isBlock = Boolean(className);

  if (!isBlock) {
    return <code className="dm-md-inline-code">{children}</code>;
  }

  const rawLang = className?.replace(/language-/, '') ?? '';
  const { html, language } = highlightCode(text, rawLang || undefined);

  return (
    <div className="dm-md-code-block not-prose">
      <div className="dm-md-code-toolbar">
        <span>{normalizeHighlightLang(language) || 'text'}</span>
        <CopyButton text={text} />
      </div>
      <pre className="dm-md-pre">
        <code
          className={`hljs ${className ?? ''}`}
          dangerouslySetInnerHTML={{ __html: html }}
        />
      </pre>
    </div>
  );
}

interface MarkdownPreviewProps {
  content: string;
  className?: string;
}

export function MarkdownPreview({ content, className }: MarkdownPreviewProps) {
  return (
    <article className={`dm-markdown max-w-3xl mx-auto ${className ?? ''}`}>
      <ReactMarkdown
        remarkPlugins={[remarkGfm]}
        components={{
          pre: ({ children }) => <>{children}</>,
          code: MarkdownCodeBlock,
          table: ({ children, ...props }) => (
            <div className="dm-md-table-wrap">
              <table className="dm-md-table" {...props}>
                {children}
              </table>
            </div>
          ),
        }}
      >
        {content}
      </ReactMarkdown>
    </article>
  );
}

interface CodePreviewProps {
  content: string;
  language?: string;
  filename?: string;
  className?: string;
}

export function CodePreview({ content, language, filename, className }: CodePreviewProps) {
  const langHint = language ?? guessLangFromFilename(filename ?? '');
  const { html, language: detected } = highlightCode(content, langHint);

  return (
    <div className={`dm-md-code-block dm-code-preview ${className ?? ''}`}>
      <div className="dm-md-code-toolbar">
        <span>{normalizeHighlightLang(detected) || 'text'}</span>
        <CopyButton text={content} />
      </div>
      <pre className="dm-md-pre">
        <code className="hljs" dangerouslySetInnerHTML={{ __html: html }} />
      </pre>
    </div>
  );
}

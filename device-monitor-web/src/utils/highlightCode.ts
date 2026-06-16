import hljs from 'highlight.js/lib/core';
import bash from 'highlight.js/lib/languages/bash';
import css from 'highlight.js/lib/languages/css';
import html from 'highlight.js/lib/languages/xml';
import javascript from 'highlight.js/lib/languages/javascript';
import json from 'highlight.js/lib/languages/json';
import markdown from 'highlight.js/lib/languages/markdown';
import python from 'highlight.js/lib/languages/python';
import rust from 'highlight.js/lib/languages/rust';
import typescript from 'highlight.js/lib/languages/typescript';
import yaml from 'highlight.js/lib/languages/yaml';

hljs.registerLanguage('bash', bash);
hljs.registerLanguage('sh', bash);
hljs.registerLanguage('shell', bash);
hljs.registerLanguage('css', css);
hljs.registerLanguage('html', html);
hljs.registerLanguage('xml', html);
hljs.registerLanguage('javascript', javascript);
hljs.registerLanguage('js', javascript);
hljs.registerLanguage('json', json);
hljs.registerLanguage('markdown', markdown);
hljs.registerLanguage('md', markdown);
hljs.registerLanguage('python', python);
hljs.registerLanguage('py', python);
hljs.registerLanguage('rust', rust);
hljs.registerLanguage('rs', rust);
hljs.registerLanguage('typescript', typescript);
hljs.registerLanguage('ts', typescript);
hljs.registerLanguage('yaml', yaml);
hljs.registerLanguage('yml', yaml);

const LANG_ALIASES: Record<string, string> = {
  shell: 'bash',
  sh: 'bash',
  js: 'javascript',
  ts: 'typescript',
  py: 'python',
  rs: 'rust',
  md: 'markdown',
  yml: 'yaml',
  html: 'xml',
};

export function normalizeHighlightLang(lang: string): string {
  const key = lang.toLowerCase();
  return LANG_ALIASES[key] ?? key;
}

export function highlightCode(text: string, lang?: string): { html: string; language: string } {
  const normalized = lang ? normalizeHighlightLang(lang) : '';
  try {
    if (normalized && hljs.getLanguage(normalized)) {
      return {
        html: hljs.highlight(text, { language: normalized }).value,
        language: normalized,
      };
    }
  } catch {
    // fall through
  }

  const auto = hljs.highlightAuto(text);
  return {
    html: auto.value,
    language: auto.language ?? 'text',
  };
}

export function guessLangFromFilename(name: string): string | undefined {
  const dot = name.lastIndexOf('.');
  if (dot <= 0) return undefined;
  const ext = name.slice(dot + 1).toLowerCase();
  const map: Record<string, string> = {
    sh: 'bash',
    bash: 'bash',
    zsh: 'bash',
    rs: 'rust',
    py: 'python',
    js: 'javascript',
    ts: 'typescript',
    json: 'json',
    yaml: 'yaml',
    yml: 'yaml',
    css: 'css',
    html: 'html',
    xml: 'xml',
    md: 'markdown',
    toml: 'toml',
    go: 'go',
    c: 'c',
    cpp: 'cpp',
    h: 'c',
  };
  return map[ext];
}

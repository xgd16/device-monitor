import { useEffect, useRef, useCallback, useState } from 'react';
import { Terminal } from '@xterm/xterm';
import { FitAddon } from '@xterm/addon-fit';
import { WebLinksAddon } from '@xterm/addon-web-links';

export type TerminalStatus = 'connecting' | 'connected' | 'disconnected' | 'exited';

const DARK_THEME = {
  background: '#1a1b26',
  foreground: '#c0caf5',
  cursor: '#c0caf5',
  selectionBackground: '#33467c',
};

const LIGHT_THEME = {
  background: '#f5f5f7',
  foreground: '#1d1d1f',
  cursor: '#1d1d1f',
  selectionBackground: '#c7d2fe',
};

export function useTerminal(
  containerRef: React.RefObject<HTMLDivElement | null>,
  theme: 'dark' | 'light',
) {
  const termRef = useRef<Terminal | null>(null);
  const fitRef = useRef<FitAddon | null>(null);
  const wsRef = useRef<WebSocket | null>(null);
  const reconnectTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const intentionalClose = useRef(false);
  const [status, setStatus] = useState<TerminalStatus>('connecting');

  const sendResize = useCallback(() => {
    const term = termRef.current;
    const ws = wsRef.current;
    if (!term || !ws || ws.readyState !== WebSocket.OPEN) return;
    fitRef.current?.fit();
    const { cols, rows } = term;
    ws.send(JSON.stringify({ type: 'resize', cols, rows }));
  }, []);

  const connect = useCallback(() => {
    if (!containerRef.current) return;

    intentionalClose.current = false;
    setStatus('connecting');

    const term = new Terminal({
      cursorBlink: true,
      fontSize: 13,
      fontFamily: 'ui-monospace, SFMono-Regular, Menlo, Consolas, monospace',
      theme: theme === 'dark' ? DARK_THEME : LIGHT_THEME,
      scrollback: 5000,
    });
    const fit = new FitAddon();
    term.loadAddon(fit);
    term.loadAddon(new WebLinksAddon());
    term.open(containerRef.current);
    fit.fit();

    termRef.current = term;
    fitRef.current = fit;

    const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
    const ws = new WebSocket(`${protocol}//${window.location.host}/ws/terminal`);
    ws.binaryType = 'arraybuffer';
    wsRef.current = ws;

    ws.onopen = () => {
      setStatus('connected');
      sendResize();
    };

    ws.onmessage = (event) => {
      if (typeof event.data === 'string') {
        try {
          const msg = JSON.parse(event.data);
          if (msg.type === 'exit') {
            setStatus('exited');
            term.writeln(`\r\n\x1b[33m[进程已退出，code=${msg.code}]\x1b[0m`);
          }
        } catch {
          term.write(event.data);
        }
      } else if (event.data instanceof ArrayBuffer) {
        term.write(new Uint8Array(event.data));
      } else if (event.data instanceof Blob) {
        event.data.arrayBuffer().then((buf) => term.write(new Uint8Array(buf)));
      }
    };

    ws.onclose = () => {
      wsRef.current = null;
      if (!intentionalClose.current) {
        setStatus('disconnected');
        reconnectTimer.current = setTimeout(connect, 3000);
      }
    };

    ws.onerror = () => {
      setStatus('disconnected');
    };

    term.onData((data) => {
      if (ws.readyState === WebSocket.OPEN) {
        ws.send(new TextEncoder().encode(data));
      }
    });

    term.onKey(({ domEvent }) => {
      if (domEvent.ctrlKey && domEvent.key === 'v') {
        domEvent.preventDefault();
        navigator.clipboard.readText().then((text) => {
          if (ws.readyState === WebSocket.OPEN) {
            ws.send(new TextEncoder().encode(text));
          }
        }).catch(() => {});
      }
    });
  }, [containerRef, theme, sendResize]);

  const disconnect = useCallback(() => {
    intentionalClose.current = true;
    if (reconnectTimer.current) {
      clearTimeout(reconnectTimer.current);
      reconnectTimer.current = null;
    }
    wsRef.current?.close();
    wsRef.current = null;
    termRef.current?.dispose();
    termRef.current = null;
    fitRef.current = null;
  }, []);

  useEffect(() => {
    connect();
    return () => disconnect();
  }, [connect, disconnect]);

  useEffect(() => {
    const term = termRef.current;
    if (!term) return;
    term.options.theme = theme === 'dark' ? DARK_THEME : LIGHT_THEME;
  }, [theme]);

  useEffect(() => {
    const el = containerRef.current;
    if (!el) return;

    const ro = new ResizeObserver(() => {
      sendResize();
    });
    ro.observe(el);
    return () => ro.disconnect();
  }, [containerRef, sendResize]);

  const paste = useCallback(async () => {
    const ws = wsRef.current;
    if (!ws || ws.readyState !== WebSocket.OPEN) return;
    try {
      const text = await navigator.clipboard.readText();
      ws.send(new TextEncoder().encode(text));
    } catch {
      // clipboard unavailable
    }
  }, []);

  const clear = useCallback(() => {
    termRef.current?.clear();
  }, []);

  return { status, paste, clear, sendResize, reconnect: connect };
}

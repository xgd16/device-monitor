import { useRef, useState, useCallback, useEffect } from 'react';
import { Card, Button, Chip } from '@heroui/react';
import { useTerminal, type TerminalStatus } from '../hooks/useTerminal';

interface Session {
  id: string;
  title: string;
}

let sessionCounter = 1;

function statusLabel(status: TerminalStatus): string {
  switch (status) {
    case 'connecting': return '连接中';
    case 'connected': return '已连接';
    case 'disconnected': return '已断开';
    case 'exited': return '已退出';
  }
}

function statusColor(status: TerminalStatus): 'success' | 'warning' | 'danger' | 'default' {
  switch (status) {
    case 'connected': return 'success';
    case 'connecting': return 'warning';
    case 'disconnected': return 'danger';
    case 'exited': return 'default';
  }
}

interface TerminalSessionProps {
  active: boolean;
  onStatus: (status: TerminalStatus) => void;
  onReady: (actions: { paste: () => void; clear: () => void; reconnect: () => void }) => void;
}

function TerminalSession({ active, onStatus, onReady }: TerminalSessionProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const { status, paste, clear, reconnect } = useTerminal(containerRef);

  useEffect(() => {
    onStatus(status);
  }, [status, onStatus]);

  useEffect(() => {
    if (active) {
      onReady({ paste, clear, reconnect });
    }
  }, [active, paste, clear, reconnect, onReady]);

  return (
    <div
      ref={containerRef}
      className={`terminal-container w-full h-full ${active ? 'block' : 'hidden'}`}
    />
  );
}

interface TerminalPanelProps {
  fullPage?: boolean;
}

export function TerminalPanel({ fullPage = false }: TerminalPanelProps) {
  const [sessions, setSessions] = useState<Session[]>([{ id: '1', title: '终端 1' }]);
  const [activeId, setActiveId] = useState('1');
  const [activeStatus, setActiveStatus] = useState<TerminalStatus>('connecting');
  const actionsRef = useRef<{ paste: () => void; clear: () => void; reconnect: () => void } | null>(null);
  const [remountKey, setRemountKey] = useState(0);

  const handleStatus = useCallback((status: TerminalStatus) => {
    setActiveStatus(status);
  }, []);

  const handleReady = useCallback((actions: { paste: () => void; clear: () => void; reconnect: () => void }) => {
    actionsRef.current = actions;
  }, []);

  const addSession = () => {
    sessionCounter += 1;
    const id = String(sessionCounter);
    setSessions((s) => [...s, { id, title: `终端 ${sessionCounter}` }]);
    setActiveId(id);
  };

  const closeSession = (id: string) => {
    setSessions((prev) => {
      if (prev.length <= 1) return prev;
      const next = prev.filter((s) => s.id !== id);
      if (activeId === id) {
        setActiveId(next[next.length - 1].id);
      }
      return next;
    });
  };

  const handleReconnect = () => {
    setRemountKey((k) => k + 1);
    actionsRef.current?.reconnect();
  };

  return (
    <Card
      className={`flex flex-col overflow-hidden ${fullPage ? 'flex-1 min-h-0 h-full' : ''}`}
      style={fullPage ? undefined : { height: '420px' }}
    >
      <div className="flex items-center gap-2 px-3 py-2 border-b border-default-200 flex-wrap shrink-0">
        <span className="text-xs font-semibold text-foreground/70">Shell</span>
        <Chip size="sm" color={statusColor(activeStatus)} variant="soft">
          {statusLabel(activeStatus)}
        </Chip>
        <div className="flex-1" />
        <div className="flex items-center gap-1 overflow-x-auto max-w-[50%]">
          {sessions.map((s) => (
            <button
              key={s.id}
              type="button"
              onClick={() => setActiveId(s.id)}
              className={`text-xs px-2 py-1 rounded-md whitespace-nowrap flex items-center gap-1 transition-colors ${
                activeId === s.id
                  ? 'bg-accent/15 text-accent font-medium'
                  : 'text-foreground/50 hover:text-foreground/80 hover:bg-default-100'
              }`}
            >
              {s.title}
              {sessions.length > 1 && (
                <span
                  role="button"
                  tabIndex={0}
                  className="opacity-50 hover:opacity-100 ml-0.5"
                  onClick={(e) => { e.stopPropagation(); closeSession(s.id); }}
                  onKeyDown={(e) => { if (e.key === 'Enter') { e.stopPropagation(); closeSession(s.id); } }}
                >
                  ×
                </span>
              )}
            </button>
          ))}
        </div>
        <Button size="sm" variant="ghost" onPress={addSession}>+ 新建</Button>
      </div>

      <div className="flex-1 min-h-0 relative bg-[#1e1e1e]">
        {sessions.map((s) => (
          <TerminalSession
            key={`${s.id}-${activeId === s.id ? remountKey : 0}`}
            active={activeId === s.id}
            onStatus={activeId === s.id ? handleStatus : () => {}}
            onReady={activeId === s.id ? handleReady : () => {}}
          />
        ))}
      </div>

      <div className="flex items-center gap-2 px-3 py-2 border-t border-default-200 shrink-0">
        <Button size="sm" variant="ghost" onPress={() => actionsRef.current?.paste()}>粘贴</Button>
        <Button size="sm" variant="ghost" onPress={() => actionsRef.current?.clear()}>清屏</Button>
        {(activeStatus === 'disconnected' || activeStatus === 'exited') && (
          <Button size="sm" variant="secondary" onPress={handleReconnect}>重连</Button>
        )}
      </div>
    </Card>
  );
}

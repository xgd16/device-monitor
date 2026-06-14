import { useEffect, useRef } from 'react';
import { useDeviceStore } from '../stores/useDeviceStore';

export function useWebSocket() {
  const wsRef = useRef<WebSocket | null>(null);
  const setData = useDeviceStore((s) => s.setData);
  const setConnected = useDeviceStore((s) => s.setConnected);
  const reconnectTimer = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => {
    const connect = () => {
      const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
      const ws = new WebSocket(`${protocol}//${window.location.host}/ws/realtime`);
      wsRef.current = ws;

      ws.onopen = () => {
        setConnected(true);
      };

      ws.onmessage = (event) => {
        try {
          const data = JSON.parse(event.data);
          setData(data);
        } catch (e) {
          console.error('Failed to parse WS message', e);
        }
      };

      ws.onclose = () => {
        setConnected(false);
        reconnectTimer.current = setTimeout(connect, 3000);
      };

      ws.onerror = () => {
        setConnected(false);
      };
    };

    connect();

    return () => {
      if (reconnectTimer.current) clearTimeout(reconnectTimer.current);
      wsRef.current?.close();
    };
  }, [setData, setConnected]);

  return wsRef;
}

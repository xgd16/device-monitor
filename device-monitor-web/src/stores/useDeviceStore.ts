import { create } from 'zustand';
import type { SystemOverview } from '../types';

interface DeviceStore {
  data: SystemOverview | null;
  history: SystemOverview[];
  connected: boolean;
  setData: (data: SystemOverview) => void;
  setConnected: (connected: boolean) => void;
}

export const useDeviceStore = create<DeviceStore>((set) => ({
  data: null,
  history: [],
  connected: false,
  setData: (data) =>
    set((state) => ({
      data,
      history: [...state.history.slice(-299), data],
    })),
  setConnected: (connected) => set({ connected }),
}));

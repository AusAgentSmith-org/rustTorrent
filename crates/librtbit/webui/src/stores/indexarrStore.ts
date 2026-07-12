import { create } from "zustand";
import {
  IndexarrIdentityStatus,
  IndexarrRecentItem,
  IndexarrSearchResult,
  IndexarrStatus,
  IndexarrSyncPreferences,
} from "../api-types";

export interface IndexarrStore {
  // Connection status
  status: IndexarrStatus | null;
  setStatus: (status: IndexarrStatus | null) => void;

  // Identity / setup
  identity: IndexarrIdentityStatus | null;
  setIdentity: (identity: IndexarrIdentityStatus | null) => void;

  syncPreferences: IndexarrSyncPreferences | null;
  setSyncPreferences: (prefs: IndexarrSyncPreferences | null) => void;

  showSetup: boolean;
  setShowSetup: (show: boolean) => void;

  // Search
  searchQuery: string;
  setSearchQuery: (query: string) => void;

  searchFilters: Record<string, string>;
  setSearchFilter: (key: string, value: string) => void;
  clearSearchFilters: () => void;

  searchResults: IndexarrSearchResult[];
  searchTotal: number;
  searchOffset: number;
  searchLoading: boolean;
  setSearchResults: (
    results: IndexarrSearchResult[],
    total: number,
    offset: number,
  ) => void;
  setSearchLoading: (loading: boolean) => void;

  // Recent torrents
  recentTorrents: IndexarrRecentItem[];
  recentLoading: boolean;
  setRecentTorrents: (torrents: IndexarrRecentItem[]) => void;
  setRecentLoading: (loading: boolean) => void;
}

export const useIndexarrStore = create<IndexarrStore>((set, get) => ({
  status: null,
  setStatus: (status) => set({ status }),

  identity: null,
  setIdentity: (identity) => set({ identity }),

  syncPreferences: null,
  setSyncPreferences: (prefs) => set({ syncPreferences: prefs }),

  showSetup: false,
  setShowSetup: (show) => set({ showSetup: show }),

  searchQuery: "",
  setSearchQuery: (query) => set({ searchQuery: query }),

  searchFilters: {},
  setSearchFilter: (key, value) =>
    set((state) => ({
      searchFilters: { ...state.searchFilters, [key]: value },
    })),
  clearSearchFilters: () => set({ searchFilters: {} }),

  searchResults: [],
  searchTotal: 0,
  searchOffset: 0,
  searchLoading: false,
  setSearchResults: (results, total, offset) =>
    set({ searchResults: results, searchTotal: total, searchOffset: offset }),
  setSearchLoading: (loading) => set({ searchLoading: loading }),

  recentTorrents: [],
  recentLoading: false,
  setRecentTorrents: (torrents) => set({ recentTorrents: torrents }),
  setRecentLoading: (loading) => set({ recentLoading: loading }),
}));

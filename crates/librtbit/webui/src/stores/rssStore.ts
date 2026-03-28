import { create } from "zustand";
import { RssFeedConfig, RssItem, RssRule } from "../api-types";

export interface RssStore {
  feeds: RssFeedConfig[];
  setFeeds: (feeds: RssFeedConfig[]) => void;

  items: RssItem[];
  setItems: (items: RssItem[]) => void;

  rules: RssRule[];
  setRules: (rules: RssRule[]) => void;

  selectedFeedFilter: string | null; // null = show all
  setSelectedFeedFilter: (feed: string | null) => void;

  loading: boolean;
  setLoading: (loading: boolean) => void;
}

export const useRssStore = create<RssStore>((set) => ({
  feeds: [],
  setFeeds: (feeds) => set({ feeds }),

  items: [],
  setItems: (items) => set({ items }),

  rules: [],
  setRules: (rules) => set({ rules }),

  selectedFeedFilter: null,
  setSelectedFeedFilter: (feed) => set({ selectedFeedFilter: feed }),

  loading: false,
  setLoading: (loading) => set({ loading }),
}));

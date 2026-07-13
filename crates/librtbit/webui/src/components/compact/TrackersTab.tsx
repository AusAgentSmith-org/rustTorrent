import { useContext, useEffect, useState } from "react";
import {
  TorrentListItem,
  TrackerAnnounceState,
  TrackerStatusEntry,
} from "../../api-types";
import { APIContext } from "../../context";
import { extractTrackers } from "../../helper/bencodeParse";
import { customSetInterval } from "../../helper/customSetInterval";
import { Spinner } from "../Spinner";

interface TrackersTabProps {
  torrent: TorrentListItem | null;
}

const STATE_LABELS: Record<TrackerAnnounceState, string> = {
  not_contacted: "Not contacted",
  updating: "Updating...",
  working: "Working",
  error: "Error",
  disabled: "Disabled",
};

const STATE_BADGE_CLASSES: Record<TrackerAnnounceState, string> = {
  not_contacted: "bg-surface-sunken text-tertiary",
  updating: "bg-warning/15 text-warning",
  working: "bg-success/15 text-success",
  error: "bg-error/15 text-error",
  disabled: "bg-surface-sunken text-tertiary",
};

const StatusBadge: React.FC<{ state: TrackerAnnounceState }> = ({ state }) => (
  <span
    className={`inline-flex items-center px-2 py-0.5 rounded-full text-xs font-medium whitespace-nowrap ${STATE_BADGE_CLASSES[state]}`}
  >
    {STATE_LABELS[state]}
  </span>
);

function formatRelativeTime(unixSecs: number | null): string {
  if (!unixSecs) return "—";
  const delta = Math.max(0, Math.floor(Date.now() / 1000) - unixSecs);
  if (delta < 5) return "just now";
  if (delta < 60) return `${delta}s ago`;
  if (delta < 3600) return `${Math.floor(delta / 60)}m ago`;
  if (delta < 86400) return `${Math.floor(delta / 3600)}h ago`;
  return `${Math.floor(delta / 86400)}d ago`;
}

const headerCell =
  "px-2 py-1.5 text-left text-xs font-semibold text-tertiary uppercase tracking-wide whitespace-nowrap";
const numCell = "px-2 py-1 text-center text-secondary tabular-nums";

export const TrackersTab: React.FC<TrackersTabProps> = ({ torrent }) => {
  const API = useContext(APIContext);
  const [entries, setEntries] = useState<TrackerStatusEntry[] | null>(null);
  const [loading, setLoading] = useState(true);
  const [unsupported, setUnsupported] = useState(false);
  // Fallback for older servers: tracker URLs parsed from torrent metadata
  const [fallbackUrls, setFallbackUrls] = useState<string[] | null>(null);

  const torrentId = torrent?.id;

  // Poll the live tracker status endpoint
  useEffect(() => {
    if (torrentId == null) return;
    setEntries(null);
    setLoading(true);
    setUnsupported(false);
    setFallbackUrls(null);

    return customSetInterval(async () => {
      try {
        const response = await API.getTrackerStatus(torrentId);
        setEntries(response.trackers);
        setLoading(false);
        return 5000;
      } catch {
        setLoading(false);
        setUnsupported(true);
        return 60000;
      }
    }, 0);
  }, [torrentId, API]);

  // Fallback: parse trackers out of the torrent metadata
  useEffect(() => {
    if (torrentId == null || !unsupported) return;
    API.getMetadata(torrentId)
      .then((data) => {
        const info = extractTrackers(data);
        const urls = new Set<string>();
        if (info.announce) urls.add(info.announce);
        for (const tier of info.announceList) {
          for (const url of tier) urls.add(url);
        }
        setFallbackUrls(Array.from(urls));
      })
      .catch(() => setFallbackUrls([]));
  }, [torrentId, unsupported, API]);

  if (!torrent) return null;

  return (
    <div className="p-3 space-y-3">
      <div className="flex items-center gap-2 text-sm">
        <span className="text-secondary font-medium">Info Hash:</span>
        <code className="bg-surface-sunken px-1.5 py-0.5 rounded text-xs font-mono">
          {torrent.info_hash}
        </code>
      </div>

      {loading && (
        <div className="flex items-center gap-2 text-sm text-tertiary">
          <Spinner />
          <span>Loading tracker status...</span>
        </div>
      )}

      {!loading && entries && entries.length === 0 && (
        <p className="text-sm text-tertiary">
          No trackers configured (DHT/PEX only)
        </p>
      )}

      {!loading && entries && entries.length > 0 && (
        <div className="overflow-x-auto">
          <table className="w-full text-sm">
            <thead>
              <tr className="border-b border-divider">
                <th className={headerCell}>Tracker</th>
                <th className={`${headerCell} text-center`}>Status</th>
                <th className={`${headerCell} text-center`}>Seeds</th>
                <th className={`${headerCell} text-center`}>Peers</th>
                <th className={`${headerCell} text-center`}>Received</th>
                <th className={`${headerCell} text-center`}>Last Announce</th>
                <th className={headerCell}>Message</th>
              </tr>
            </thead>
            <tbody>
              {entries.map((t) => (
                <tr
                  key={t.url}
                  className="border-b border-divider/50 hover:bg-surface-raised"
                >
                  <td
                    className="px-2 py-1 font-mono text-xs max-w-[26rem] truncate"
                    title={t.url}
                  >
                    {t.url}
                  </td>
                  <td className="px-2 py-1 text-center">
                    <StatusBadge state={t.state} />
                  </td>
                  <td className={numCell}>{t.seeders ?? "—"}</td>
                  <td className={numCell}>{t.leechers ?? "—"}</td>
                  <td className={numCell}>{t.peers_returned ?? "—"}</td>
                  <td className={numCell}>
                    {formatRelativeTime(t.last_announce_unix)}
                  </td>
                  <td
                    className="px-2 py-1 text-xs text-tertiary max-w-[18rem] truncate"
                    title={t.last_error ?? undefined}
                  >
                    {t.last_error ?? ""}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}

      {/* Older servers without /trackers endpoint: static list from metadata */}
      {unsupported && (
        <div>
          <p className="text-xs text-tertiary mb-2">
            Live tracker status not available on this server; showing configured
            trackers.
          </p>
          {fallbackUrls === null && (
            <div className="flex items-center gap-2 text-sm text-tertiary">
              <Spinner />
              <span>Loading tracker info...</span>
            </div>
          )}
          {fallbackUrls && fallbackUrls.length === 0 && (
            <p className="text-sm text-tertiary">
              No trackers found (DHT/PEX only)
            </p>
          )}
          {fallbackUrls && fallbackUrls.length > 0 && (
            <div className="space-y-1">
              {fallbackUrls.map((url) => (
                <div
                  key={url}
                  className="flex items-center gap-2 text-sm font-mono bg-surface-sunken px-2 py-1 rounded"
                >
                  <span className="truncate text-primary" title={url}>
                    {url}
                  </span>
                </div>
              ))}
            </div>
          )}
        </div>
      )}
    </div>
  );
};

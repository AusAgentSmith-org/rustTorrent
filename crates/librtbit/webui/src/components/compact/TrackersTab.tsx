import { useContext, useEffect, useState } from "react";
import { TorrentListItem } from "../../api-types";
import { APIContext } from "../../context";
import { extractTrackers, TorrentTrackerInfo } from "../../helper/bencodeParse";
import { Spinner } from "../Spinner";

interface TrackersTabProps {
  torrent: TorrentListItem | null;
}

export const TrackersTab: React.FC<TrackersTabProps> = ({ torrent }) => {
  const API = useContext(APIContext);
  const [trackerInfo, setTrackerInfo] = useState<TorrentTrackerInfo | null>(
    null,
  );
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!torrent) return;
    setLoading(true);
    setError(null);
    setTrackerInfo(null);

    API.getMetadata(torrent.id)
      .then((data) => {
        const info = extractTrackers(data);
        setTrackerInfo(info);
      })
      .catch(() => {
        setError("Could not load tracker metadata");
      })
      .finally(() => setLoading(false));
  }, [torrent?.id, API]);

  if (!torrent) return null;

  // Collect all unique tracker URLs
  const allTrackerUrls = new Set<string>();
  if (trackerInfo?.announce) {
    allTrackerUrls.add(trackerInfo.announce);
  }
  if (trackerInfo?.announceList) {
    for (const tier of trackerInfo.announceList) {
      for (const url of tier) {
        allTrackerUrls.add(url);
      }
    }
  }

  return (
    <div className="p-3 space-y-3">
      <div className="text-sm">
        <div className="flex items-center gap-2 mb-2">
          <span className="text-secondary font-medium">Info Hash:</span>
          <code className="bg-surface-sunken px-1.5 py-0.5 rounded text-xs font-mono">
            {torrent.info_hash}
          </code>
        </div>
      </div>

      <div>
        <h3 className="text-sm font-medium text-secondary mb-2">Trackers</h3>
        {loading && (
          <div className="flex items-center gap-2 text-sm text-tertiary">
            <Spinner />
            <span>Loading tracker info...</span>
          </div>
        )}
        {error && <p className="text-sm text-tertiary">{error}</p>}
        {trackerInfo && allTrackerUrls.size === 0 && !loading && (
          <p className="text-sm text-tertiary">
            No trackers found (DHT/PEX only)
          </p>
        )}
        {allTrackerUrls.size > 0 && (
          <div className="space-y-1">
            {Array.from(allTrackerUrls).map((url) => (
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

      {trackerInfo && trackerInfo.announceList.length > 1 && (
        <div>
          <h3 className="text-sm font-medium text-secondary mb-2">
            Tracker Tiers
          </h3>
          <div className="space-y-2">
            {trackerInfo.announceList.map((tier, i) => (
              <div key={i}>
                <span className="text-xs text-tertiary">Tier {i + 1}</span>
                {tier.map((url) => (
                  <div
                    key={url}
                    className="text-sm font-mono text-secondary pl-3"
                  >
                    {url}
                  </div>
                ))}
              </div>
            ))}
          </div>
        </div>
      )}
    </div>
  );
};

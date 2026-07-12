import { formatBytes } from "../helper/formatBytes";
import { formatSecondsToTime } from "../helper/formatSecondsToTime";
import { useStatsStore } from "../stores/statsStore";

const FooterPiece: React.FC<{ children: React.ReactNode }> = ({ children }) => {
  return <div className="p-1">{children}</div>;
};

export const Footer: React.FC = () => {
  const stats = useStatsStore((stats) => stats.stats);
  return (
    <div className="bg-surface-raised text-nowrap text-sm font-medium text-secondary flex gap-x-1 lg:gap-x-5 justify-evenly flex-wrap">
      <FooterPiece>
        &darr; {stats.download_speed.human_readable} (
        {formatBytes(stats.counters.fetched_bytes)})
      </FooterPiece>
      <FooterPiece>
        &uarr; {stats.upload_speed.human_readable} (
        {formatBytes(stats.counters.uploaded_bytes)})
      </FooterPiece>
      <FooterPiece>Peers: {stats.peers.live}</FooterPiece>
      <FooterPiece>up {formatSecondsToTime(stats.uptime_seconds)}</FooterPiece>
      <FooterPiece>
        <a
          href="/swagger/"
          target="_blank"
          className="text-blue-600 dark:text-blue-400 hover:underline"
        >
          API
        </a>
      </FooterPiece>
    </div>
  );
};

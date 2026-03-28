import {
  JSX,
  useCallback,
  useContext,
  useEffect,
  useRef,
  useState,
} from "react";
import {
  AddTorrentResponse,
  ErrorDetails as ApiErrorDetails,
} from "./api-types";
import { APIContext } from "./context";
import { RootContent } from "./components/RootContent";
import { customSetInterval } from "./helper/customSetInterval";
import { LogStreamModal } from "./components/modal/LogStreamModal";
import { Toolbar } from "./components/Toolbar";
import { Sidebar } from "./components/Sidebar";
import { useTorrentStore } from "./stores/torrentStore";
import { useErrorStore } from "./stores/errorStore";
import { AlertModal } from "./components/modal/AlertModal";
import { useStatsStore } from "./stores/statsStore";
import { Footer } from "./components/Footer";
import { FileSelectionModal } from "./components/modal/FileSelectionModal";
import { MultiTorrentUploadModal } from "./components/modal/MultiTorrentUploadModal";
import { useIsLargeScreen } from "./hooks/useIsLargeScreen";
import { useUIStore } from "./stores/uiStore";

export interface ErrorWithLabel {
  text: string;
  details?: ApiErrorDetails;
}

export interface ContextType {
  setCloseableError: (error: ErrorWithLabel | null) => void;
  refreshTorrents: () => void;
}

type PendingUpload =
  | { type: "single"; file: File }
  | { type: "multi"; files: File[] }
  | null;

export const RtbitWebUI = (props: {
  title: string;
  version: string;
  menuButtons?: JSX.Element[];
}) => {
  const [logsOpened, setLogsOpened] = useState<boolean>(false);
  const setOtherError = useErrorStore((state) => state.setOtherError);

  const API = useContext(APIContext);

  const isLargeScreen = useIsLargeScreen();
  const sidebarOpen = useUIStore((state) => state.sidebarOpen);
  const setSidebarOpen = useUIStore((state) => state.setSidebarOpen);
  const currentPage = useUIStore((state) => state.currentPage);

  const setTorrents = useTorrentStore((state) => state.setTorrents);
  const setTorrentsLoading = useTorrentStore(
    (state) => state.setTorrentsLoading,
  );
  const setRefreshTorrents = useTorrentStore(
    (state) => state.setRefreshTorrents,
  );

  const refreshTorrents = async (): Promise<number> => {
    setTorrentsLoading(true);
    try {
      const response = await API.listTorrents({ withStats: true });
      setTorrents(response.torrents);
      setOtherError(null);

      // Determine polling interval based on torrent states
      // Fast poll (1s) if any torrent is live/initializing, slow poll (5s) otherwise
      const hasActiveTorrents = response.torrents.some(
        (t) => t.stats?.state === "live" || t.stats?.state === "initializing",
      );
      return hasActiveTorrents ? 1000 : 5000;
    } catch (e) {
      setOtherError({ text: "Error refreshing torrents", details: e as any });
      console.error(e);
      return 5000;
    } finally {
      setTorrentsLoading(false);
    }
  };

  const setStats = useStatsStore((state) => state.setStats);

  // Register the refresh callback
  useEffect(() => {
    setRefreshTorrents(refreshTorrents as unknown as () => void);
  }, []);

  useEffect(() => {
    return customSetInterval(async () => refreshTorrents(), 0);
  }, []);

  useEffect(() => {
    return customSetInterval(
      async () =>
        API.stats().then(
          (stats) => {
            setStats(stats);
            return 1000;
          },
          (e) => {
            console.error(e);
            return 5000;
          },
        ),
      0,
    );
  }, []);

  // --- Drag and drop ---
  const [isDragging, setIsDragging] = useState(false);
  const dragCounterRef = useRef(0);

  const [pendingUpload, setPendingUpload] = useState<PendingUpload>(null);

  // Single-file upload state (mirrors UploadButton logic)
  const [listTorrentResponse, setListTorrentResponse] =
    useState<AddTorrentResponse | null>(null);
  const [listTorrentLoading, setListTorrentLoading] = useState(false);
  const [listTorrentError, setListTorrentError] =
    useState<ErrorWithLabel | null>(null);

  // When a single file is pending, call list_only API
  useEffect(() => {
    if (pendingUpload?.type !== "single") return;

    const file = pendingUpload.file;
    setListTorrentLoading(true);
    setListTorrentResponse(null);
    setListTorrentError(null);

    let cancelled = false;
    const t = setTimeout(async () => {
      try {
        const response = await API.uploadTorrent(file, { list_only: true });
        if (!cancelled) setListTorrentResponse(response);
      } catch (e) {
        if (!cancelled)
          setListTorrentError({
            text: "Error listing torrent files",
            details: e as ApiErrorDetails,
          });
      } finally {
        if (!cancelled) setListTorrentLoading(false);
      }
    }, 0);

    return () => {
      cancelled = true;
      clearTimeout(t);
    };
  }, [pendingUpload]);

  const clearSingleUpload = () => {
    setPendingUpload(null);
    setListTorrentResponse(null);
    setListTorrentError(null);
    setListTorrentLoading(false);
  };

  const handleDragEnter = useCallback((e: React.DragEvent) => {
    e.preventDefault();
    e.stopPropagation();
    dragCounterRef.current++;
    if (dragCounterRef.current === 1) {
      setIsDragging(true);
    }
  }, []);

  const handleDragLeave = useCallback((e: React.DragEvent) => {
    e.preventDefault();
    e.stopPropagation();
    dragCounterRef.current--;
    if (dragCounterRef.current === 0) {
      setIsDragging(false);
    }
  }, []);

  const handleDragOver = useCallback((e: React.DragEvent) => {
    e.preventDefault();
    e.stopPropagation();
  }, []);

  const handleDrop = useCallback((e: React.DragEvent) => {
    e.preventDefault();
    e.stopPropagation();
    dragCounterRef.current = 0;
    setIsDragging(false);

    const files = Array.from(e.dataTransfer.files).filter((f) =>
      f.name.endsWith(".torrent"),
    );

    if (files.length === 0) return;

    if (files.length === 1) {
      setPendingUpload({ type: "single", file: files[0] });
    } else {
      setPendingUpload({ type: "multi", files });
    }
  }, []);

  // Handler for FileInput multi-file selection (via Toolbar)
  const handleMultiFileSelect = useCallback((files: File[]) => {
    if (files.length === 1) {
      setPendingUpload({ type: "single", file: files[0] });
    } else if (files.length > 1) {
      setPendingUpload({ type: "multi", files });
    }
  }, []);

  return (
    <div className="bg-surface h-dvh flex flex-col overflow-hidden">
      <Toolbar
        title={props.title}
        version={props.version}
        onMultiFileSelect={handleMultiFileSelect}
        onLogsClick={() => setLogsOpened(true)}
        menuButtons={props.menuButtons}
      />

      <div className="flex-1 min-h-0 flex flex-row">
        {/* Sidebar - only on large screens, only on torrents page */}
        {isLargeScreen && currentPage === "torrents" && <Sidebar />}

        {/* Main content area */}
        <div
          className="flex-1 min-h-0 relative"
          onDragEnter={handleDragEnter}
          onDragLeave={handleDragLeave}
          onDragOver={handleDragOver}
          onDrop={handleDrop}
        >
          <RootContent />
          {isDragging && (
            <div className="absolute inset-0 z-50 flex items-center justify-center bg-surface/90">
              <div className="border-2 border-dashed border-primary rounded-lg p-8 text-center">
                <p className="text-lg font-semibold text-primary">
                  Drop .torrent files here
                </p>
              </div>
            </div>
          )}
        </div>
      </div>

      {/* Mobile sidebar drawer */}
      {!isLargeScreen && sidebarOpen && (
        <div className="fixed inset-0 z-40">
          <div
            className="absolute inset-0 bg-black/50"
            onClick={() => setSidebarOpen(false)}
          />
          <div className="absolute left-0 top-0 bottom-0 w-64 bg-surface shadow-xl">
            <Sidebar />
          </div>
        </div>
      )}

      <Footer />

      <LogStreamModal show={logsOpened} onClose={() => setLogsOpened(false)} />
      <AlertModal />

      {pendingUpload?.type === "single" && (
        <FileSelectionModal
          onHide={clearSingleUpload}
          listTorrentResponse={listTorrentResponse}
          listTorrentError={listTorrentError}
          listTorrentLoading={listTorrentLoading}
          data={pendingUpload.file}
        />
      )}

      {pendingUpload?.type === "multi" && (
        <MultiTorrentUploadModal
          files={pendingUpload.files}
          onHide={() => setPendingUpload(null)}
        />
      )}
    </div>
  );
};

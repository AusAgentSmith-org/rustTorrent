import {
  JSX,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useState,
} from "react";
import { FaPause, FaPlay, FaTrash } from "react-icons/fa";
import { BsCollection, BsGlobe2, BsRss } from "react-icons/bs";
import { GoSearch, GoX } from "react-icons/go";
import {
  BsBodyText,
  BsBoxArrowRight,
  BsMoon,
  BsSliders2,
  BsSun,
} from "react-icons/bs";
import { HiOutlineMenu } from "react-icons/hi";
import debounce from "lodash.debounce";

// @ts-expect-error - SVG import handled by vite-plugin-svgr
import Logo from "../../assets/logo.svg?react";

import { APIContext } from "../context";
import { useUIStore } from "../stores/uiStore";
import { useTorrentStore } from "../stores/torrentStore";
import { useErrorStore } from "../stores/errorStore";
import { useIsLargeScreen } from "../hooks/useIsLargeScreen";
import { useKeyboardShortcuts } from "../hooks/useKeyboardShortcuts";
import {
  ErrorDetails,
  STATE_LIVE,
  STATE_PAUSED,
  TorrentListItem,
} from "../api-types";
import { DarkMode } from "../helper/darkMode";
import { useAuthStore } from "../stores/authStore";
import { AuthAPI, IndexarrAPI } from "../http-api";
import { useIndexarrStore } from "../stores/indexarrStore";
import { MagnetInput } from "./buttons/MagnetInput";
import { FileInput } from "./buttons/FileInput";
import { ConfigModal } from "./config/ConfigModal";
import { DeleteTorrentModal } from "./modal/DeleteTorrentModal";

interface ToolbarProps {
  title: string;
  version: string;
  onMultiFileSelect?: (files: File[]) => void;
  onLogsClick: () => void;
  menuButtons?: JSX.Element[];
}

export const Toolbar: React.FC<ToolbarProps> = ({
  title,
  version,
  onMultiFileSelect,
  onLogsClick,
  menuButtons,
}) => {
  const API = useContext(APIContext);
  const isLargeScreen = useIsLargeScreen();

  // UI store
  const selectedTorrentIds = useUIStore((state) => state.selectedTorrentIds);
  const searchQuery = useUIStore((state) => state.searchQuery);
  const setSearchQuery = useUIStore((state) => state.setSearchQuery);
  const setSidebarOpen = useUIStore((state) => state.setSidebarOpen);
  const currentPage = useUIStore((s) => s.currentPage);
  const setCurrentPage = useUIStore((s) => s.setCurrentPage);

  // Torrent store
  const torrents = useTorrentStore((state) => state.torrents);
  const refreshTorrents = useTorrentStore((state) => state.refreshTorrents);

  // Error store
  const setCloseableError = useErrorStore((state) => state.setCloseableError);

  // Indexarr
  const indexarrEnabled = useIndexarrStore((s) => s.status?.enabled ?? false);
  const setIndexarrStatus = useIndexarrStore((s) => s.setStatus);

  useEffect(() => {
    IndexarrAPI.getStatus()
      .then(setIndexarrStatus)
      .catch(() => setIndexarrStatus({ enabled: false }));
  }, []);

  // Local state
  const [disabled, setDisabled] = useState(false);
  const [showDeleteModal, setShowDeleteModal] = useState(false);
  const [torrentsToDelete, setTorrentsToDelete] = useState<
    Pick<TorrentListItem, "id" | "name">[]
  >([]);
  const [localSearch, setLocalSearch] = useState(searchQuery);
  const [isDark, setIsDark] = useState(DarkMode.isDark());
  const [configOpen, setConfigOpen] = useState(false);
  const authState = useAuthStore((s) => s.state);
  const refreshToken = useAuthStore((s) => s.refreshToken);
  const clearTokens = useAuthStore((s) => s.clearTokens);

  const handleLogout = async () => {
    if (refreshToken) {
      try {
        await AuthAPI.logout(refreshToken);
      } catch {
        // Ignore logout API errors — clear local state regardless
      }
    }
    clearTokens();
  };

  // Debounced search
  // eslint-disable-next-line react-hooks/exhaustive-deps
  const debouncedSetSearch = useCallback(
    debounce((value: string) => setSearchQuery(value), 150),
    [setSearchQuery],
  );

  const handleSearchChange = (e: React.ChangeEvent<HTMLInputElement>) => {
    const value = e.target.value;
    setLocalSearch(value);
    debouncedSetSearch(value);
  };

  const clearSearch = () => {
    setLocalSearch("");
    setSearchQuery("");
  };

  const handleDarkModeToggle = () => {
    DarkMode.toggle();
    setIsDark(DarkMode.isDark());
  };

  const selectedCount = selectedTorrentIds.size;
  const hasSelection = selectedCount > 0;

  const getTorrentById = (id: number) => torrents?.find((t) => t.id === id);

  const openDeleteModal = useCallback(() => {
    const torrentsList = Array.from(selectedTorrentIds).map((id) => {
      const torrent = getTorrentById(id);
      return {
        id,
        name: torrent?.name ?? null,
      };
    });
    setTorrentsToDelete(torrentsList);
    setShowDeleteModal(true);
  }, [selectedTorrentIds, torrents]);

  // Keyboard shortcuts (global)
  const keyboardActions = useMemo(
    () => ({ onDelete: openDeleteModal }),
    [openDeleteModal],
  );
  useKeyboardShortcuts(keyboardActions);

  const runBulkAction = async (
    action: (id: number) => Promise<void>,
    skipState: string,
    errorLabel: string,
  ) => {
    setDisabled(true);
    try {
      for (const id of selectedTorrentIds) {
        const torrent = getTorrentById(id);
        if (torrent?.stats?.state === skipState) continue;
        try {
          await action(id);
          refreshTorrents();
        } catch (e) {
          setCloseableError({
            text: `Error ${errorLabel} torrent id=${id}`,
            details: e as ErrorDetails,
          });
        }
      }
    } finally {
      setDisabled(false);
    }
  };

  const pauseSelected = () =>
    runBulkAction((id) => API.pause(id), STATE_PAUSED, "pausing");
  const resumeSelected = () =>
    runBulkAction((id) => API.start(id), STATE_LIVE, "starting");

  // Hide built-in configure button when custom menuButtons are provided
  const showBuiltInConfigButton = !menuButtons || menuButtons.length === 0;

  const showTorrentActions = currentPage === "torrents";

  const navTab =
    "inline-flex items-center gap-1.5 px-3 py-1.5 text-sm font-medium rounded-lg transition-colors cursor-pointer whitespace-nowrap";
  const navTabActive = `${navTab} bg-primary/12 text-primary`;
  const navTabInactive = `${navTab} text-secondary hover:text-text hover:bg-surface/50`;

  const toolbarIcon =
    "p-1.5 text-secondary hover:text-text hover:bg-surface/50 rounded-lg cursor-pointer transition-colors inline-flex items-center justify-center";
  const toolbarIconDisabled =
    "p-1.5 text-tertiary rounded-lg inline-flex items-center justify-center opacity-40 cursor-not-allowed";
  const toolbarIconDanger =
    "p-1.5 text-error hover:text-error hover:bg-error/10 rounded-lg cursor-pointer transition-colors inline-flex items-center justify-center";
  const toolbarIconDangerDisabled =
    "p-1.5 text-error rounded-lg inline-flex items-center justify-center opacity-40 cursor-not-allowed";

  return (
    <div className="bg-surface-raised drop-shadow-lg">
      {/* ── Single unified row ── */}
      <div className="flex items-center gap-1 px-2 py-1.5">
        {/* Mobile hamburger */}
        {!isLargeScreen && (
          <button
            onClick={() => {
              setSidebarOpen(true);
              setCurrentPage("torrents");
            }}
            className="p-1.5 text-secondary hover:text-primary cursor-pointer"
            title="Open sidebar"
          >
            <HiOutlineMenu className="w-5 h-5" />
          </button>
        )}

        {/* Logo + title */}
        <div className="flex items-center gap-1.5 mr-1">
          <Logo className="w-6 h-6" alt="logo" />
          <h1 className="hidden lg:flex items-center">
            <span className="text-lg font-bold">{title}</span>
          </h1>
        </div>

        {/* Divider after logo */}
        {isLargeScreen && <div className="w-px h-5 bg-divider mx-1" />}

        {/* Page navigation tabs (pill style) */}
        {isLargeScreen && (
          <div className="flex items-center gap-0.5">
            <button
              onClick={() => setCurrentPage("torrents")}
              className={
                currentPage === "torrents" ? navTabActive : navTabInactive
              }
            >
              <BsCollection className="w-3.5 h-3.5" />
              Torrents
            </button>
            {indexarrEnabled && (
              <button
                onClick={() => setCurrentPage("indexarr")}
                className={
                  currentPage === "indexarr" ? navTabActive : navTabInactive
                }
              >
                <BsGlobe2 className="w-3.5 h-3.5" />
                Index
              </button>
            )}
            <button
              onClick={() => setCurrentPage("rss")}
              className={currentPage === "rss" ? navTabActive : navTabInactive}
            >
              <BsRss className="w-3.5 h-3.5" />
              RSS
            </button>
          </div>
        )}

        {/* Spacer */}
        <div className="flex-1" />

        {/* Search input (torrents page only) */}
        {showTorrentActions && (
          <div className="relative hidden lg:block">
            <GoSearch className="absolute left-2.5 top-1/2 -translate-y-1/2 w-3.5 h-3.5 text-tertiary" />
            <input
              type="text"
              data-search-input
              value={localSearch}
              onChange={handleSearchChange}
              placeholder="Filter..."
              className="pl-8 pr-7 py-1.5 w-44 text-sm bg-surface/50 border border-divider rounded-lg focus:outline-none focus:border-primary placeholder:text-tertiary"
            />
            {localSearch && (
              <button
                onClick={clearSearch}
                className="absolute right-1.5 top-1/2 -translate-y-1/2 p-0.5 text-tertiary hover:text-secondary rounded cursor-pointer"
              >
                <GoX className="w-3.5 h-3.5" />
              </button>
            )}
          </div>
        )}

        {/* Divider before actions */}
        {showTorrentActions && (
          <div className="hidden lg:block w-px h-5 bg-divider mx-1" />
        )}

        {/* Torrent action group */}
        {showTorrentActions && (
          <div className="flex items-center gap-0.5 bg-surface/30 rounded-lg p-0.5">
            <MagnetInput className="grow-0 justify-center !px-2.5 !py-1 !text-sm !rounded-lg" />
            <FileInput
              className="grow-0 justify-center !px-2.5 !py-1 !text-sm !rounded-lg"
              onMultiFileSelect={onMultiFileSelect}
            />

            <div className="w-px h-5 bg-divider mx-0.5" />

            <button
              onClick={resumeSelected}
              disabled={disabled || !hasSelection}
              className={
                disabled || !hasSelection ? toolbarIconDisabled : toolbarIcon
              }
              title="Resume selected"
            >
              <FaPlay className="w-3 h-3" />
            </button>
            <button
              onClick={pauseSelected}
              disabled={disabled || !hasSelection}
              className={
                disabled || !hasSelection ? toolbarIconDisabled : toolbarIcon
              }
              title="Pause selected"
            >
              <FaPause className="w-3 h-3" />
            </button>
            <button
              onClick={openDeleteModal}
              disabled={disabled || !hasSelection}
              className={
                disabled || !hasSelection
                  ? toolbarIconDangerDisabled
                  : toolbarIconDanger
              }
              title="Delete selected"
            >
              <FaTrash className="w-3 h-3" />
            </button>

            {hasSelection && (
              <span className="hidden lg:inline text-xs text-secondary ml-0.5 mr-1">
                {selectedCount}
              </span>
            )}
          </div>
        )}

        {/* Divider before utility icons */}
        <div className="hidden lg:block w-px h-5 bg-divider mx-1" />

        {/* Utility icons */}
        {menuButtons?.map((b, i) => (
          <span key={i}>{b}</span>
        ))}
        {showBuiltInConfigButton && (
          <>
            <button
              onClick={() => setConfigOpen(true)}
              className={toolbarIcon}
              title="Configure"
            >
              <BsSliders2 className="w-4 h-4" />
            </button>
            <ConfigModal
              isOpen={configOpen}
              onClose={() => setConfigOpen(false)}
              version={version}
            />
          </>
        )}
        <button onClick={onLogsClick} className={toolbarIcon} title="View logs">
          <BsBodyText className="w-4 h-4" />
        </button>
        <button
          onClick={handleDarkModeToggle}
          className={toolbarIcon}
          title="Toggle dark mode"
        >
          {isDark ? (
            <BsSun className="w-4 h-4" />
          ) : (
            <BsMoon className="w-4 h-4" />
          )}
        </button>
        {authState === "authenticated" && (
          <button onClick={handleLogout} className={toolbarIcon} title="Logout">
            <BsBoxArrowRight className="w-4 h-4" />
          </button>
        )}
      </div>

      {/* Delete modal */}
      <DeleteTorrentModal
        show={showDeleteModal}
        onHide={() => setShowDeleteModal(false)}
        torrents={torrentsToDelete}
      />
    </div>
  );
};

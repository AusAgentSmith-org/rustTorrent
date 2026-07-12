import { useMemo, useCallback, useEffect, useState, useRef } from "react";
import { Virtuoso } from "react-virtuoso";
import { TorrentListItem } from "../../api-types";
import { TorrentTableRow } from "./TorrentTableRow";
import { useUIStore } from "../../stores/uiStore";
import { useColumnStore, ColumnDef, ColumnId } from "../../stores/columnStore";
import { Spinner } from "../Spinner";
import { SortIcon } from "../SortIcon";
import { isTorrentVisible, SortDirection } from "../../helper/torrentFilters";
import { ColumnMenu } from "./ColumnMenu";
import { TorrentContextMenu, ContextMenuState } from "../TorrentContextMenu";

// Sort columns: all sortable column IDs
export type TableSortColumn =
  | "id"
  | "name"
  | "size"
  | "progress"
  | "downloadedBytes"
  | "downSpeed"
  | "upSpeed"
  | "uploadedBytes"
  | "eta"
  | "peers"
  | "state"
  | "ratio"
  | "category"
  | "seeding_time"
  | "queue_position"
  | "availability";

const DEFAULT_SORT_COLUMN: TableSortColumn = "id";
const DEFAULT_SORT_DIRECTION: SortDirection = "desc";

function getTableSortValue(
  t: TorrentListItem,
  column: TableSortColumn,
): number | string {
  switch (column) {
    case "id":
      return t.id;
    case "name":
      return (t.name ?? "").toLowerCase();
    case "size":
      return t.stats?.total_bytes ?? 0;
    case "progress":
      return t.stats?.total_bytes
        ? (t.stats.progress_bytes ?? 0) / t.stats.total_bytes
        : 0;
    case "downloadedBytes":
      return t.stats?.progress_bytes ?? 0;
    case "downSpeed":
      return t.stats?.live?.download_speed?.mbps ?? 0;
    case "upSpeed":
      return t.stats?.live?.upload_speed?.mbps ?? 0;
    case "uploadedBytes":
      return t.stats?.live?.snapshot.uploaded_bytes ?? 0;
    case "eta": {
      if (!t.stats?.live) return Infinity;
      const remaining =
        (t.stats.total_bytes ?? 0) - (t.stats.progress_bytes ?? 0);
      const speed = t.stats.live.download_speed?.mbps ?? 0;
      if (speed <= 0 || remaining <= 0) return remaining <= 0 ? 0 : Infinity;
      return remaining / (speed * 1024 * 1024);
    }
    case "peers":
      return t.stats?.live?.snapshot.peer_stats?.live ?? 0;
    case "state":
      return t.stats?.state ?? "";
    case "ratio": {
      if (t.stats?.ratio != null) return t.stats.ratio;
      const uploaded = t.stats?.live?.snapshot.uploaded_bytes ?? 0;
      const total = t.stats?.total_bytes ?? 1;
      return total > 0 ? uploaded / total : 0;
    }
    case "category":
      return (t.category ?? "").toLowerCase();
    case "seeding_time":
      return t.stats?.seeding_time_secs ?? 0;
    case "queue_position":
      return t.stats?.queue_position ?? Infinity;
    case "availability":
      return t.stats?.min_piece_availability ?? 0;
  }
}

/** Generate a <colgroup> from visible columns with their widths */
function TableColGroup({ columns }: { columns: ColumnDef[] }) {
  // Subscribe to columnWidths directly so we re-render when widths change
  useColumnStore((s) => s.columnWidths);
  const getWidth = useColumnStore((s) => s.getWidth);
  return (
    <colgroup>
      {columns.map((col) => {
        const w = getWidth(col.id);
        return (
          <col key={col.id} style={w > 0 ? { width: `${w}px` } : undefined} />
        );
      })}
    </colgroup>
  );
}

interface TorrentTableProps {
  torrents: TorrentListItem[] | null;
  loading: boolean;
}

export const TorrentTable: React.FC<TorrentTableProps> = ({
  torrents,
  loading,
}) => {
  const selectedTorrentIds = useUIStore((state) => state.selectedTorrentIds);
  const selectTorrent = useUIStore((state) => state.selectTorrent);
  const toggleSelection = useUIStore((state) => state.toggleSelection);
  const selectRange = useUIStore((state) => state.selectRange);
  const selectRelative = useUIStore((state) => state.selectRelative);
  const selectAll = useUIStore((state) => state.selectAll);
  const clearSelection = useUIStore((state) => state.clearSelection);
  const searchQuery = useUIStore((state) => state.searchQuery);
  const statusFilter = useUIStore((state) => state.statusFilter);
  const categoryFilter = useUIStore((state) => state.categoryFilter);

  // Subscribe to data directly so component re-renders on changes
  useColumnStore((s) => s.columnVisibility);
  useColumnStore((s) => s.columnWidths);
  useColumnStore((s) => s.columnOrder);
  const visibleColumns = useColumnStore((s) => s.getVisibleColumns)();
  const getWidth = useColumnStore((s) => s.getWidth);
  const setColumnWidth = useColumnStore((s) => s.setColumnWidth);

  const normalizedQuery = searchQuery.toLowerCase().trim();

  // Local sorting state
  const [sortColumn, setSortColumnState] =
    useState<TableSortColumn>(DEFAULT_SORT_COLUMN);
  const [sortDirection, setSortDirectionState] = useState<SortDirection>(
    DEFAULT_SORT_DIRECTION,
  );

  // Resize state
  const [resizing, setResizing] = useState<{
    colId: ColumnId;
    startX: number;
    startWidth: number;
  } | null>(null);

  // Column context menu
  const [contextMenu, setContextMenu] = useState<{
    x: number;
    y: number;
  } | null>(null);

  // Torrent right-click context menu
  const [torrentContextMenu, setTorrentContextMenu] =
    useState<ContextMenuState | null>(null);

  const setSortColumn = useCallback((column: TableSortColumn) => {
    setSortColumnState((prevColumn) => {
      setSortDirectionState((prevDir) => {
        const newDir: SortDirection =
          prevColumn === column ? (prevDir === "asc" ? "desc" : "asc") : "desc";
        return newDir;
      });
      return column;
    });
  }, []);

  // Sort and filter torrents for virtualization
  const filteredTorrents = useMemo(() => {
    if (!torrents) return null;

    return [...torrents]
      .filter((t) =>
        isTorrentVisible(t, normalizedQuery, statusFilter, categoryFilter),
      )
      .sort((a, b) => {
        const aVal = getTableSortValue(a, sortColumn);
        const bVal = getTableSortValue(b, sortColumn);
        const cmp =
          typeof aVal === "string"
            ? aVal.localeCompare(bVal as string)
            : (aVal as number) - (bVal as number);
        return sortDirection === "asc" ? cmp : -cmp;
      });
  }, [
    torrents,
    normalizedQuery,
    statusFilter,
    categoryFilter,
    sortColumn,
    sortDirection,
  ]);

  // Compute visible IDs for keyboard navigation
  const visibleTorrentIds = useMemo(() => {
    if (!filteredTorrents) return [];
    return filteredTorrents.map((t) => t.id);
  }, [filteredTorrents]);

  const allSelected = !!(
    visibleTorrentIds.length > 0 &&
    visibleTorrentIds.every((id) => selectedTorrentIds.has(id))
  );
  const someSelected = visibleTorrentIds.some((id) =>
    selectedTorrentIds.has(id),
  );

  const handleHeaderCheckbox = () => {
    if (allSelected) {
      clearSelection();
    } else {
      selectAll(visibleTorrentIds);
    }
  };

  const handleSort = (column: TableSortColumn) => {
    setSortColumn(column);
  };

  // Store orderedIds in a ref so handleRowClick doesn't need it as a dependency
  const orderedIdsRef = useRef<number[]>([]);
  orderedIdsRef.current = visibleTorrentIds;

  // Handle keyboard navigation
  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      const activeElement = document.activeElement;
      if (
        activeElement &&
        (activeElement.tagName === "INPUT" ||
          activeElement.tagName === "TEXTAREA" ||
          activeElement.tagName === "SELECT")
      ) {
        return;
      }

      if (e.key === "ArrowDown") {
        e.preventDefault();
        selectRelative("down", orderedIdsRef.current);
      } else if (e.key === "ArrowUp") {
        e.preventDefault();
        selectRelative("up", orderedIdsRef.current);
      }
    };

    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [selectRelative]);

  // Row click handler — skip right-clicks (handled by context menu)
  const handleRowClick = useCallback(
    (id: number, e: React.MouseEvent) => {
      if (e.button === 2) return;
      if (e.shiftKey) {
        e.preventDefault();
        selectRange(id, orderedIdsRef.current);
      } else {
        selectTorrent(id);
      }
    },
    [selectRange, selectTorrent],
  );

  // Row right-click handler
  const handleRowContextMenu = useCallback(
    (id: number, e: React.MouseEvent) => {
      const torrent = filteredTorrents?.find((t) => t.id === id);
      if (!torrent) return;

      let selected: TorrentListItem[];
      if (selectedTorrentIds.has(id)) {
        // Right-clicked on an already-selected torrent: operate on all selected
        selected = filteredTorrents!.filter((t) =>
          selectedTorrentIds.has(t.id),
        );
      } else {
        // Right-clicked on an unselected torrent: select only this one
        selectTorrent(id);
        selected = [torrent];
      }

      setTorrentContextMenu({
        x: e.clientX,
        y: e.clientY,
        torrent,
        selectedTorrents: selected,
      });
    },
    [filteredTorrents, selectedTorrentIds, selectTorrent],
  );

  // Column resize handlers
  useEffect(() => {
    if (!resizing) return;

    const handleMouseMove = (e: MouseEvent) => {
      const delta = e.clientX - resizing.startX;
      setColumnWidth(resizing.colId, resizing.startWidth + delta);
    };

    const handleMouseUp = () => {
      setResizing(null);
    };

    document.addEventListener("mousemove", handleMouseMove);
    document.addEventListener("mouseup", handleMouseUp);
    // Prevent text selection while resizing
    document.body.style.userSelect = "none";
    document.body.style.cursor = "col-resize";

    return () => {
      document.removeEventListener("mousemove", handleMouseMove);
      document.removeEventListener("mouseup", handleMouseUp);
      document.body.style.userSelect = "";
      document.body.style.cursor = "";
    };
  }, [resizing, setColumnWidth]);

  const handleResizeStart = useCallback(
    (colId: ColumnId, e: React.MouseEvent) => {
      e.preventDefault();
      e.stopPropagation();
      let currentWidth = getWidth(colId);
      if (currentWidth === 0) {
        // Flex column: use actual rendered width
        const th = (e.target as HTMLElement).closest("th");
        if (th) currentWidth = th.getBoundingClientRect().width;
      }
      setResizing({ colId, startX: e.clientX, startWidth: currentWidth });
    },
    [getWidth],
  );

  const handleHeaderContextMenu = useCallback((e: React.MouseEvent) => {
    e.preventDefault();
    setContextMenu({ x: e.clientX, y: e.clientY });
  }, []);

  // Item renderer for react-virtuoso
  const itemContent = useCallback(
    (index: number) => {
      const torrent = filteredTorrents![index];
      return (
        <TorrentTableRow
          key={torrent.id}
          torrent={torrent}
          isSelected={selectedTorrentIds.has(torrent.id)}
          onRowClick={handleRowClick}
          onContextMenu={handleRowContextMenu}
          onCheckboxChange={toggleSelection}
          visibleColumns={visibleColumns}
        />
      );
    },
    [
      filteredTorrents,
      selectedTorrentIds,
      handleRowClick,
      handleRowContextMenu,
      toggleSelection,
      visibleColumns,
    ],
  );

  if (loading) {
    return (
      <div className="flex justify-center items-center h-64">
        <Spinner />
      </div>
    );
  }

  if (!torrents || torrents.length === 0) {
    return (
      <div className="flex flex-col items-center justify-center h-64 text-tertiary">
        <p className="text-lg">No torrents</p>
        <p className="">Add a torrent to get started</p>
      </div>
    );
  }

  return (
    <div className="flex flex-col h-full">
      {/* Header */}
      <table className="w-full table-fixed">
        <TableColGroup columns={visibleColumns} />
        <thead className="bg-surface-raised text-sm">
          <tr
            className="border-b border-divider"
            onContextMenu={handleHeaderContextMenu}
          >
            {visibleColumns.map((col) => {
              if (col.id === "checkbox") {
                return (
                  <th
                    key="checkbox"
                    className="px-2 py-3 border-r border-divider/40"
                  >
                    <input
                      type="checkbox"
                      checked={allSelected}
                      ref={(el) => {
                        if (el) el.indeterminate = someSelected && !allSelected;
                      }}
                      onChange={handleHeaderCheckbox}
                      className="w-4 h-4 rounded border-divider-strong bg-surface text-primary focus:ring-primary"
                    />
                  </th>
                );
              }
              if (col.id === "status_icon") {
                return (
                  <th
                    key="status_icon"
                    className="px-1 py-3 border-r border-divider/40"
                  />
                );
              }

              const alignClass =
                col.align === "center"
                  ? "text-center"
                  : col.align === "right"
                    ? "text-right"
                    : "text-left";
              const isSortable = col.sortable;
              const canResize = col.configurable;

              return (
                <th
                  key={col.id}
                  className={`relative px-2 py-2 text-secondary select-none whitespace-nowrap border-r border-divider/40 ${alignClass} ${isSortable ? "cursor-pointer hover:text-text" : ""}`}
                  onClick={
                    isSortable
                      ? () => handleSort(col.id as TableSortColumn)
                      : undefined
                  }
                >
                  {col.label}
                  {isSortable && (
                    <SortIcon
                      column={col.id}
                      sortColumn={sortColumn}
                      sortDirection={sortDirection}
                    />
                  )}
                  {canResize && (
                    <div
                      className="absolute right-0 top-0 bottom-0 w-1.5 cursor-col-resize hover:bg-primary/40 z-10"
                      onMouseDown={(e) =>
                        handleResizeStart(col.id as ColumnId, e)
                      }
                    />
                  )}
                </th>
              );
            })}
          </tr>
        </thead>
      </table>
      {/* Virtualized body */}
      <div className="flex-1 min-h-0">
        <Virtuoso
          totalCount={filteredTorrents?.length ?? 0}
          itemContent={itemContent}
        />
      </div>

      {/* Column visibility context menu */}
      {contextMenu && (
        <ColumnMenu
          x={contextMenu.x}
          y={contextMenu.y}
          onClose={() => setContextMenu(null)}
        />
      )}

      {/* Torrent right-click context menu */}
      {torrentContextMenu && (
        <TorrentContextMenu
          menu={torrentContextMenu}
          onClose={() => setTorrentContextMenu(null)}
        />
      )}
    </div>
  );
};

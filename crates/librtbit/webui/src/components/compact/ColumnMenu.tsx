import { useEffect, useRef } from "react";
import {
  COLUMN_DEFS,
  useColumnStore,
  ColumnId,
} from "../../stores/columnStore";
import { GoTriangleUp, GoTriangleDown } from "react-icons/go";

interface ColumnMenuProps {
  x: number;
  y: number;
  onClose: () => void;
}

export const ColumnMenu: React.FC<ColumnMenuProps> = ({ x, y, onClose }) => {
  // Subscribe to data directly so component re-renders on changes
  const columnVisibility = useColumnStore((s) => s.columnVisibility);
  const columnOrder = useColumnStore((s) => s.columnOrder);
  const toggleColumnVisibility = useColumnStore(
    (s) => s.toggleColumnVisibility,
  );
  const moveColumn = useColumnStore((s) => s.moveColumn);
  const resetColumns = useColumnStore((s) => s.resetColumns);
  const menuRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const handleClick = (e: MouseEvent) => {
      if (menuRef.current && !menuRef.current.contains(e.target as Node)) {
        onClose();
      }
    };
    const handleEsc = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    document.addEventListener("mousedown", handleClick);
    document.addEventListener("keydown", handleEsc);
    return () => {
      document.removeEventListener("mousedown", handleClick);
      document.removeEventListener("keydown", handleEsc);
    };
  }, [onClose]);

  // Clamp position to viewport
  const menuWidth = 260;
  const menuMaxHeight = 400;
  const left = Math.min(x, window.innerWidth - menuWidth - 8);
  const top = Math.min(y, window.innerHeight - menuMaxHeight - 8);

  // Show columns in their current order
  const orderedColumns = columnOrder
    .map((id) => COLUMN_DEFS.find((c) => c.id === id)!)
    .filter((c) => c && c.configurable);

  const isVisible = (id: ColumnId): boolean => {
    const v = columnVisibility[id];
    if (v !== undefined) return v;
    return COLUMN_DEFS.find((c) => c.id === id)?.defaultVisible ?? true;
  };

  const arrowBtn =
    "p-0.5 text-tertiary hover:text-primary rounded hover:bg-surface-raised disabled:opacity-30 disabled:cursor-not-allowed cursor-pointer";

  return (
    <div
      ref={menuRef}
      className="fixed z-50 bg-surface-raised border border-divider rounded-lg shadow-lg py-1 overflow-y-auto"
      style={{ left, top, width: menuWidth, maxHeight: menuMaxHeight }}
    >
      <div className="px-3 py-1.5 text-xs font-semibold text-tertiary uppercase tracking-wider border-b border-divider">
        Columns
      </div>
      {orderedColumns.map((col, idx) => {
        const visible = isVisible(col.id);
        return (
          <div
            key={col.id}
            className="flex items-center gap-1 px-2 py-1 hover:bg-surface"
          >
            <label className="flex items-center gap-2 flex-1 cursor-pointer text-sm">
              <input
                type="checkbox"
                checked={visible}
                onChange={() => toggleColumnVisibility(col.id as ColumnId)}
                className="w-3.5 h-3.5 rounded border-divider-strong bg-surface text-primary focus:ring-primary"
              />
              <span className={visible ? "text-primary" : "text-tertiary"}>
                {col.label}
              </span>
            </label>
            <div className="flex items-center gap-0.5 shrink-0">
              <button
                className={arrowBtn}
                onClick={() => moveColumn(col.id as ColumnId, "up")}
                disabled={idx === 0}
                title="Move up"
              >
                <GoTriangleUp className="w-3.5 h-3.5" />
              </button>
              <button
                className={arrowBtn}
                onClick={() => moveColumn(col.id as ColumnId, "down")}
                disabled={idx === orderedColumns.length - 1}
                title="Move down"
              >
                <GoTriangleDown className="w-3.5 h-3.5" />
              </button>
            </div>
          </div>
        );
      })}
      <div className="border-t border-divider mt-1 pt-1">
        <button
          onClick={() => {
            resetColumns();
            onClose();
          }}
          className="w-full text-left px-3 py-1.5 text-xs text-secondary hover:bg-surface cursor-pointer"
        >
          Reset to defaults
        </button>
      </div>
    </div>
  );
};

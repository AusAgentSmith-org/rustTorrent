import React, { useContext, useState } from "react";
import { APIContext } from "../../context";
import { Button } from "../buttons/Button";

export const AdvancedTab: React.FC = () => {
  const API = useContext(APIContext);
  const [rustLog, setRustLog] = useState("");
  const [applying, setApplying] = useState(false);
  const [status, setStatus] = useState<string | null>(null);

  const handleApply = async () => {
    setApplying(true);
    setStatus(null);
    try {
      await API.setRustLog(rustLog);
      setStatus("Applied successfully");
    } catch {
      setStatus("Error applying RUST_LOG");
    } finally {
      setApplying(false);
    }
  };

  return (
    <div className="space-y-4 py-2">
      <div>
        <label className="block text-sm text-secondary mb-1">RUST_LOG</label>
        <div className="flex gap-2">
          <input
            type="text"
            value={rustLog}
            onChange={(e) => setRustLog(e.target.value)}
            placeholder="e.g. librtbit=debug,info"
            className="flex-1 px-3 py-1.5 text-sm bg-surface border border-divider rounded focus:outline-none focus:border-primary"
          />
          <Button
            variant="primary"
            onClick={handleApply}
            disabled={applying || !rustLog}
          >
            Apply
          </Button>
        </div>
        {status && <p className="text-sm mt-1 text-secondary">{status}</p>}
      </div>
      <div className="text-secondary text-xs">
        Change the logging level at runtime. Examples: &quot;info&quot;,
        &quot;librtbit=debug&quot;, &quot;librtbit::session=trace&quot;
      </div>
    </div>
  );
};

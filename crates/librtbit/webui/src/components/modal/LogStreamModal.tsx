import { useContext } from "react";
import { APIContext } from "../../context";
import { ErrorComponent } from "../ErrorComponent";
import { LogStream } from "../LogStream";
import { Modal } from "./Modal";
import { ModalFooter } from "./ModalFooter";
import { Button } from "../buttons/Button";

interface Props {
  show: boolean;
  onClose: () => void;
}

export const LogStreamModal: React.FC<Props> = ({ show, onClose }) => {
  const api = useContext(APIContext);
  const logsUrl = api.getStreamLogsUrl();

  return (
    <Modal
      isOpen={show}
      onClose={onClose}
      title="rtbit server logs"
      className="max-w-7xl"
    >
      <div className="p-3 border-b dark:border-slate-500 h-[70vh]">
        {logsUrl ? (
          <LogStream url={logsUrl} />
        ) : (
          <ErrorComponent
            error={{ text: "HTTP API not available to stream logs" }}
          />
        )}
      </div>
      <ModalFooter>
        <Button variant="primary" onClick={onClose}>
          Close
        </Button>
      </ModalFooter>
    </Modal>
  );
};

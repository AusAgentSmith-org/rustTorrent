import {
  MdCheckCircle,
  MdDownload,
  MdError,
  MdOutlineMotionPhotosPaused,
  MdOutlineUpload,
  MdQueue,
} from "react-icons/md";

type Props = {
  className?: string;
  finished: boolean;
  live: boolean;
  error: boolean;
  queued?: boolean;
};

export const StatusIcon = ({
  className,
  finished,
  live,
  error,
  queued,
}: Props) => {
  const isSeeding = finished && live;
  if (error) return <MdError className={`text-error ${className}`} />;
  if (queued) return <MdQueue className={`text-warning ${className}`} />;
  if (isSeeding)
    return <MdOutlineUpload className={`text-success ${className}`} />;
  if (finished)
    return <MdCheckCircle className={`text-success ${className}`} />;
  if (live) return <MdDownload className={`text-primary ${className}`} />;
  else
    return (
      <MdOutlineMotionPhotosPaused className={`text-secondary ${className}`} />
    );
};

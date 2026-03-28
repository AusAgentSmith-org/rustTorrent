export function loopUntilSuccess<T>(
  callback: () => Promise<T>,
  interval: number,
): () => void {
  let timeoutId: any;

  const executeCallback = async () => {
    const retry = await callback().then(
      () => false,
      () => true,
    );
    if (retry) {
      scheduleNext();
    }
  };

  const scheduleNext = (overrideInterval?: number) => {
    timeoutId = setTimeout(
      executeCallback,
      overrideInterval !== undefined ? overrideInterval : interval,
    );
  };

  scheduleNext(0);

  return () => clearTimeout(timeoutId);
}

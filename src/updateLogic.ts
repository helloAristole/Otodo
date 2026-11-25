export type UpdateDownloadEvent =
  | { event: "Started"; data: { contentLength?: number } }
  | { event: "Progress"; data: { chunkLength: number } }
  | { event: "Finished" };

export type UpdateDownloadState = {
  downloadedBytes: number;
  totalBytes: number | null;
  finished: boolean;
};

export function createUpdateDownload(): UpdateDownloadState {
  return {
    downloadedBytes: 0,
    totalBytes: null,
    finished: false,
  };
}

export function advanceUpdateDownload(
  current: UpdateDownloadState,
  event: UpdateDownloadEvent,
): UpdateDownloadState {
  if (event.event === "Started") {
    return {
      downloadedBytes: 0,
      totalBytes: event.data.contentLength ?? null,
      finished: false,
    };
  }

  if (event.event === "Progress") {
    return {
      ...current,
      downloadedBytes: current.downloadedBytes + event.data.chunkLength,
    };
  }

  return {
    ...current,
    finished: true,
  };
}

export function getUpdateProgressPercent(
  state: UpdateDownloadState,
): number | null {
  if (state.finished) return 100;
  if (!state.totalBytes || state.totalBytes <= 0) return null;

  const percent = Math.round(
    (state.downloadedBytes / state.totalBytes) * 100,
  );
  return Math.min(99, Math.max(0, percent));
}

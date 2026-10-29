/**
 * Video tiles without live <video> elements. A browser keeps a video's HTTP connection
 * open for as long as the element exists (and often after it's removed), so a grid of
 * video tiles can tie up every connection to the server and stall everything else. Each
 * tile instead grabs one frame, at most a couple at a time, and lets the video go.
 */

/**
 * The server makes video thumbnails when it has ffmpeg; frames are only grabbed in
 * the browser when it doesn't (learned from the first video thumbnail that fails).
 */
export const serverFrames = { available: true };

/** Matroska videos come repackaged as a stream the browser can't seek: see StreamPlayer. */
export function streamed(name: string): boolean {
  return /\.mkv$/i.test(name);
}

const MAX_FRAMES = 3000;
const CONCURRENT = 2;
const TIMEOUT_MS = 20000;
const FRAME_WIDTH = 480;

/** Captured frames by video URL, as object URLs (oldest first). */
const frames = new Map<string, string>();
const waiting: { url: string; resolve: (frame: string | null) => void; cancelled: boolean }[] = [];
let running = 0;

export function cachedFrame(url: string): string | undefined {
  return frames.get(url);
}

/** Resolves to a frame of the video at `url`, or null if it can't be read. */
export function frameOf(url: string): { promise: Promise<string | null>; cancel: () => void } {
  const cached = frames.get(url);
  if (cached) return { promise: Promise.resolve(cached), cancel: () => {} };
  let entry!: (typeof waiting)[number];
  const promise = new Promise<string | null>((resolve) => {
    entry = { url, resolve, cancelled: false };
    waiting.push(entry);
  });
  pump();
  return {
    promise,
    cancel: () => {
      entry.cancelled = true;
    },
  };
}

function pump() {
  while (running < CONCURRENT && waiting.length) {
    const job = waiting.shift()!;
    if (job.cancelled) {
      job.resolve(null);
      continue;
    }
    running++;
    grab(job.url)
      .then((frame) => job.resolve(frame))
      .finally(() => {
        running--;
        pump();
      });
  }
}

/** Stops a media element downloading and frees its connection. */
export function releaseMedia(video: HTMLMediaElement) {
  video.pause();
  video.removeAttribute('src');
  video.load();
}

/** Svelte action: when a playing <video> goes away, stop its download too. */
export function releaseOnDestroy(video: HTMLMediaElement) {
  return {
    destroy() {
      releaseMedia(video);
    },
  };
}

async function grab(url: string): Promise<string | null> {
  const cached = frames.get(url);
  if (cached) return cached;
  const video = document.createElement('video');
  video.muted = true;
  video.playsInline = true;
  video.preload = 'auto';
  try {
    const frame = await new Promise<string | null>((resolve) => {
      const timer = setTimeout(() => resolve(null), TIMEOUT_MS);
      const done = (value: string | null) => {
        clearTimeout(timer);
        resolve(value);
      };
      video.onerror = () => done(null);
      video.onloadedmetadata = () => {
        // A little way in, so it isn't a black fade-in frame.
        video.currentTime = Math.min(0.5, (video.duration || 1) / 2);
      };
      video.onseeked = () => {
        const width = Math.min(FRAME_WIDTH, video.videoWidth || FRAME_WIDTH);
        const height = Math.round((width * (video.videoHeight || 9)) / (video.videoWidth || 16));
        const canvas = document.createElement('canvas');
        canvas.width = width;
        canvas.height = height;
        try {
          canvas.getContext('2d')!.drawImage(video, 0, 0, width, height);
          canvas.toBlob((blob) => done(blob ? URL.createObjectURL(blob) : null), 'image/jpeg', 0.8);
        } catch {
          done(null);
        }
      };
      video.src = url;
    });
    if (frame) {
      frames.set(url, frame);
      if (frames.size > MAX_FRAMES) {
        const [oldest, oldestFrame] = frames.entries().next().value!;
        frames.delete(oldest);
        URL.revokeObjectURL(oldestFrame);
      }
    }
    return frame;
  } finally {
    releaseMedia(video);
  }
}

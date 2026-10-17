/** Slideshow preferences, remembered per browser. */
export interface SlideshowSettings {
  /** Seconds each photo is shown. */
  interval: number;
  shuffle: boolean;
  videos: boolean;
  subfolders: boolean;
}

const STORAGE_KEY = 'pixelgrove.slideshow';
export const INTERVALS = [3, 5, 10, 15, 30, 60];

function load(): SlideshowSettings {
  const defaults: SlideshowSettings = { interval: 5, shuffle: false, videos: true, subfolders: false };
  try {
    return { ...defaults, ...JSON.parse(localStorage.getItem(STORAGE_KEY) ?? '{}') };
  } catch {
    return defaults;
  }
}

export const slideshowSettings: SlideshowSettings = $state(load());

export function saveSlideshowSettings(): void {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(slideshowSettings));
  } catch {
    // Private mode or storage disabled: settings just aren't remembered.
  }
}

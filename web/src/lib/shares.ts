import type { Cover } from './media.svelte';

export interface ShareView {
  id: number;
  token: string;
  url: string;
  kind: 'media' | 'folder';
  name: string;
  path: string;
  cover: Cover | null;
  folder_id: number | null;
  media_id: number | null;
  created_at: number;
  created_by: string;
  expires_at: number | null;
  has_password: boolean;
  allow_download: boolean;
  views: number;
  last_viewed_at: number | null;
}

/** The link to hand out: the server's public URL if configured, else this origin. */
export function fullUrl(share: { url: string }): string {
  return share.url.startsWith('/') ? `${location.origin}${share.url}` : share.url;
}

/** Copies text, falling back to a hidden textarea where the Clipboard API is
 * unavailable (plain-HTTP LAN addresses). */
export async function copyText(text: string): Promise<boolean> {
  try {
    if (navigator.clipboard?.writeText) {
      await navigator.clipboard.writeText(text);
      return true;
    }
  } catch {
    // Fall through to the legacy method.
  }
  const area = document.createElement('textarea');
  area.value = text;
  area.style.position = 'fixed';
  area.style.opacity = '0';
  document.body.append(area);
  area.select();
  const ok = document.execCommand('copy');
  area.remove();
  return ok;
}

export const expiryChoices: { label: string; days: number | null }[] = [
  { label: 'Never', days: null },
  { label: '1 day', days: 1 },
  { label: '7 days', days: 7 },
  { label: '30 days', days: 30 },
  { label: '90 days', days: 90 },
  { label: '1 year', days: 365 },
];

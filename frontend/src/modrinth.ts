const MODRINTH_BASE = 'https://api.modrinth.com/v2';
const USER_AGENT = 'IR77-ContentInstaller/1.0.0 (panel.ir77.gg)';

function headers(): Record<string, string> {
  return { 'User-Agent': USER_AGENT };
}

// ─── Types ───────────────────────────────────────────────────

export interface ModrinthProject {
  slug: string;
  title: string;
  description: string;
  project_type: 'mod' | 'modpack' | 'resourcepack' | 'shader' | 'plugin' | 'datapack';
  project_id: string;
  author: string;
  icon_url: string | null;
  color: number | null;
  downloads: number;
  follows: number;
  categories: string[];
  versions: string[];
  client_side: string;
  server_side: string;
  date_created: string;
  date_modified: string;
  latest_version: string | null;
  license: string;
  gallery: string[];
}

export interface ModrinthSearchResponse {
  hits: ModrinthProject[];
  offset: number;
  limit: number;
  total_hits: number;
}

export interface ModrinthVersionFile {
  url: string;
  filename: string;
  size: number;
  primary: boolean;
  hashes: {
    sha1?: string;
    sha512?: string;
  };
}

export interface ModrinthDependency {
  version_id: string | null;
  project_id: string | null;
  file_name: string | null;
  dependency_type: 'required' | 'optional' | 'incompatible' | 'embedded';
}

export interface ModrinthVersion {
  id: string;
  project_id: string;
  author_id: string;
  name: string;
  version_number: string;
  loaders: string[];
  game_versions: string[];
  version_type: 'release' | 'beta' | 'alpha';
  featured: boolean;
  status: string;
  downloads: number;
  date_published: string;
  files: ModrinthVersionFile[];
  dependencies: ModrinthDependency[];
  changelog: string | null;
}

export interface ModrinthProjectDetails {
  id: string;
  slug: string;
  title: string;
  description: string;
  body: string;
  project_type: string;
  icon_url: string | null;
  downloads: number;
  followers: number;
  categories: string[];
  versions: string[];
  loaders: string[];
  game_versions: string[];
  license: { id: string; name: string; url: string | null };
  source_url: string | null;
  issues_url: string | null;
  wiki_url: string | null;
  discord_url: string | null;
  donation_urls: Array<{ id: string; platform: string; url: string }>;
  gallery: Array<{ url: string; title: string | null; description: string | null }>;
  date_created: string;
  date_modified: string;
}

export type SearchIndex = 'relevance' | 'downloads' | 'follows' | 'newest' | 'updated';

// ─── API Functions ───────────────────────────────────────────

/**
 * Search for projects on Modrinth.
 */
export async function searchProjects(opts: {
  query?: string;
  projectType?: 'mod' | 'plugin' | 'datapack';
  loaders?: string[];
  gameVersions?: string[];
  categories?: string[];
  index?: SearchIndex;
  offset?: number;
  limit?: number;
}): Promise<ModrinthSearchResponse> {
  const params = new URLSearchParams();
  if (opts.query) params.set('query', opts.query);
  if (opts.index) params.set('index', opts.index);
  params.set('offset', String(opts.offset ?? 0));
  params.set('limit', String(opts.limit ?? 20));

  // Build facets: arrays within the outer array are AND'd, values within inner arrays are OR'd
  const facets: string[][] = [];

  if (opts.projectType) {
    facets.push([`project_type:${opts.projectType}`]);
  }
  if (opts.loaders && opts.loaders.length > 0) {
    facets.push(opts.loaders.map((l) => `categories:${l}`));
  }
  if (opts.gameVersions && opts.gameVersions.length > 0) {
    facets.push(opts.gameVersions.map((v) => `versions:${v}`));
  }
  if (opts.categories && opts.categories.length > 0) {
    facets.push(opts.categories.map((c) => `categories:${c}`));
  }
  // Always filter for server-side support
  facets.push(['server_side:required', 'server_side:optional']);

  if (facets.length > 0) {
    params.set('facets', JSON.stringify(facets));
  }

  const res = await fetch(`${MODRINTH_BASE}/search?${params}`, { headers: headers() });
  if (!res.ok) throw new Error(`Modrinth search failed: ${res.status}`);
  return res.json();
}

/**
 * Get full project details.
 */
export async function getProject(idOrSlug: string): Promise<ModrinthProjectDetails> {
  const res = await fetch(`${MODRINTH_BASE}/project/${idOrSlug}`, { headers: headers() });
  if (!res.ok) throw new Error(`Modrinth project fetch failed: ${res.status}`);
  return res.json();
}

/**
 * List versions for a project, optionally filtered by loader and game version.
 */
export async function getProjectVersions(
  idOrSlug: string,
  opts?: { loaders?: string[]; gameVersions?: string[] },
): Promise<ModrinthVersion[]> {
  const params = new URLSearchParams();
  if (opts?.loaders && opts.loaders.length > 0) {
    params.set('loaders', JSON.stringify(opts.loaders));
  }
  if (opts?.gameVersions && opts.gameVersions.length > 0) {
    params.set('game_versions', JSON.stringify(opts.gameVersions));
  }

  const res = await fetch(`${MODRINTH_BASE}/project/${idOrSlug}/version?${params}`, { headers: headers() });
  if (!res.ok) throw new Error(`Modrinth versions fetch failed: ${res.status}`);
  return res.json();
}

/**
 * Look up a version by file hash (sha1 or sha512).
 * Used to identify installed files.
 */
export async function getVersionFromHash(
  hash: string,
  algorithm: 'sha1' | 'sha512' = 'sha1',
): Promise<ModrinthVersion | null> {
  try {
    const res = await fetch(`${MODRINTH_BASE}/version_file/${hash}?algorithm=${algorithm}`, { headers: headers() });
    if (res.status === 404) return null;
    if (!res.ok) return null;
    return res.json();
  } catch {
    return null;
  }
}

/**
 * Batch lookup versions from multiple file hashes.
 */
export async function getVersionsFromHashes(
  hashes: string[],
  algorithm: 'sha1' | 'sha512' = 'sha1',
): Promise<Record<string, ModrinthVersion>> {
  if (hashes.length === 0) return {};
  const res = await fetch(`${MODRINTH_BASE}/version_files`, {
    method: 'POST',
    headers: { ...headers(), 'Content-Type': 'application/json' },
    body: JSON.stringify({ hashes, algorithm }),
  });
  if (!res.ok) return {};
  return res.json();
}

/**
 * Get multiple projects by IDs.
 */
export async function getProjects(ids: string[]): Promise<ModrinthProjectDetails[]> {
  if (ids.length === 0) return [];
  const params = new URLSearchParams();
  params.set('ids', JSON.stringify(ids));
  const res = await fetch(`${MODRINTH_BASE}/projects?${params}`, { headers: headers() });
  if (!res.ok) return [];
  return res.json();
}

// ─── Helpers ─────────────────────────────────────────────────

/** Format download count for display */
export function formatDownloads(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}K`;
  return String(n);
}

/** Get the primary file from a version's files array */
export function getPrimaryFile(version: ModrinthVersion): ModrinthVersionFile | null {
  return version.files.find((f) => f.primary) ?? version.files[0] ?? null;
}

/** Format file size */
export function formatSize(bytes: number): string {
  if (bytes >= 1_048_576) return `${(bytes / 1_048_576).toFixed(1)} MB`;
  if (bytes >= 1024) return `${(bytes / 1024).toFixed(0)} KB`;
  return `${bytes} B`;
}

/** Time ago string */
export function timeAgo(dateStr: string): string {
  const diff = Date.now() - new Date(dateStr).getTime();
  const minutes = Math.floor(diff / 60000);
  if (minutes < 60) return `${minutes}m ago`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours}h ago`;
  const days = Math.floor(hours / 24);
  if (days < 30) return `${days}d ago`;
  const months = Math.floor(days / 30);
  if (months < 12) return `${months}mo ago`;
  return `${Math.floor(months / 12)}y ago`;
}

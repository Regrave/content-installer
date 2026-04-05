// ---- Types ----

export interface CurseForgeProject {
  id: number;
  name: string;
  slug: string;
  summary: string;
  downloadCount: number;
  classId: number;
  authors: Array<{ id: number; name: string }>;
  logo: { thumbnailUrl: string; url: string } | null;
  allowModDistribution: boolean | null;
  dateReleased: string;
  dateModified: string;
}

export interface CurseForgeSearchResponse {
  data: CurseForgeProject[];
  pagination: {
    index: number;
    pageSize: number;
    resultCount: number;
    totalCount: number;
  };
}

export interface CurseForgeFile {
  id: number;
  modId: number;
  displayName: string;
  fileName: string;
  releaseType: number; // 1=Release, 2=Beta, 3=Alpha
  fileStatus: number;
  fileDate: string;
  fileLength: number;
  downloadCount: number;
  downloadUrl: string | null;
  gameVersions: string[];
  isServerPack: boolean;
  serverPackFileId: number | null;
}

export interface CurseForgeFilesResponse {
  data: CurseForgeFile[];
  pagination: {
    index: number;
    pageSize: number;
    resultCount: number;
    totalCount: number;
  };
}

// ---- CurseForge class IDs ----
export const CF_CLASS_MODS = 6;
export const CF_CLASS_PLUGINS = 5;
export const CF_CLASS_DATAPACKS = 4559;
export const CF_CLASS_MODPACKS = 4471;

// ---- Mod loader type enum ----
export const CF_LOADER_ANY = 0;
export const CF_LOADER_FORGE = 1;
export const CF_LOADER_FABRIC = 4;
export const CF_LOADER_QUILT = 5;
export const CF_LOADER_NEOFORGE = 6;

// ---- Sort fields ----
export type CurseForgeSortField = 1 | 2 | 3 | 4 | 5 | 6;

// ---- API Functions (proxied through panel backend) ----

export async function searchCurseForge(serverUuid: string, opts: {
  searchFilter?: string;
  classId?: number;
  gameVersion?: string;
  modLoaderType?: number;
  sortField?: number;
  sortOrder?: string;
  index?: number;
  pageSize?: number;
}): Promise<CurseForgeSearchResponse> {
  const baseUrl = `/api/client/servers/${serverUuid}/content-installer/curseforge`;
  const params = new URLSearchParams();
  if (opts.searchFilter) params.set('searchFilter', opts.searchFilter);
  if (opts.classId) params.set('classId', String(opts.classId));
  if (opts.gameVersion) params.set('gameVersion', opts.gameVersion);
  if (opts.modLoaderType !== undefined && opts.modLoaderType > 0) {
    params.set('modLoaderType', String(opts.modLoaderType));
  }
  if (opts.sortField) params.set('sortField', String(opts.sortField));
  if (opts.sortOrder) params.set('sortOrder', opts.sortOrder);
  params.set('index', String(opts.index ?? 0));
  params.set('pageSize', String(opts.pageSize ?? 20));

  const res = await fetch(`${baseUrl}/search?${params}`);
  if (!res.ok) throw new Error(`CurseForge search failed: ${res.status}`);
  return res.json();
}

export async function getCurseForgeFiles(serverUuid: string, opts: {
  modId: number;
  gameVersion?: string;
  modLoaderType?: number;
  index?: number;
  pageSize?: number;
}): Promise<CurseForgeFilesResponse> {
  const baseUrl = `/api/client/servers/${serverUuid}/content-installer/curseforge`;
  const params = new URLSearchParams();
  params.set('modId', String(opts.modId));
  if (opts.gameVersion) params.set('gameVersion', opts.gameVersion);
  if (opts.modLoaderType !== undefined && opts.modLoaderType > 0) {
    params.set('modLoaderType', String(opts.modLoaderType));
  }
  params.set('index', String(opts.index ?? 0));
  params.set('pageSize', String(opts.pageSize ?? 20));

  const res = await fetch(`${baseUrl}/files?${params}`);
  if (!res.ok) throw new Error(`CurseForge files failed: ${res.status}`);
  return res.json();
}

export async function getCurseForgeDescription(serverUuid: string, modId: number): Promise<string> {
  const baseUrl = `/api/client/servers/${serverUuid}/content-installer/curseforge`;
  const params = new URLSearchParams({ modId: String(modId) });
  const res = await fetch(`${baseUrl}/description?${params}`);
  if (!res.ok) return '';
  const data = await res.json();
  return data.data ?? '';
}

export async function checkCurseForgeStatus(serverUuid: string): Promise<boolean> {
  try {
    const res = await fetch(`/api/client/servers/${serverUuid}/content-installer/curseforge/status`);
    if (!res.ok) return false;
    const data = await res.json();
    return data.configured === true;
  } catch {
    return false;
  }
}

// ---- Helpers ----

export function formatDownloads(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}K`;
  return String(n);
}

export function formatSize(bytes: number): string {
  if (bytes >= 1_048_576) return `${(bytes / 1_048_576).toFixed(1)} MB`;
  if (bytes >= 1024) return `${(bytes / 1024).toFixed(0)} KB`;
  return `${bytes} B`;
}

export function releaseTypeLabel(type: number): string {
  if (type === 1) return 'release';
  if (type === 2) return 'beta';
  if (type === 3) return 'alpha';
  return 'unknown';
}

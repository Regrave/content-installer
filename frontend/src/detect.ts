import { axiosInstance } from '@/api/axios.ts';

export type ServerPlatform = 'vanilla' | 'plugins' | 'mods' | 'both' | 'unknown';
export type ServerLoader =
  | 'vanilla'
  | 'paper' | 'spigot' | 'bukkit' | 'purpur' | 'folia' | 'pufferfish' | 'leaves'
  | 'fabric' | 'forge' | 'neoforge' | 'quilt'
  | 'mohist' | 'arclight' | 'sponge'
  | 'unknown';

export interface ServerDetection {
  platform: ServerPlatform;
  loader: ServerLoader;
  mcVersion: string | null;
  hasPluginsDir: boolean;
  hasModsDir: boolean;
  /** The primary world directory name (from server.properties level-name, defaults to "world") */
  worldDir: string;
  /** All detected world directories (contain level.dat) */
  worldDirs: string[];
}

/** Map from loader to Modrinth loader facet values */
export const LOADER_TO_MODRINTH: Record<ServerLoader, string[]> = {
  vanilla: ['datapack'],
  paper: ['paper', 'spigot', 'bukkit'],
  spigot: ['spigot', 'bukkit'],
  bukkit: ['bukkit'],
  purpur: ['purpur', 'paper', 'spigot', 'bukkit'],
  pufferfish: ['paper', 'spigot', 'bukkit'],
  folia: ['folia', 'paper'],
  leaves: ['paper', 'spigot', 'bukkit'],
  fabric: ['fabric'],
  forge: ['forge'],
  neoforge: ['neoforge'],
  quilt: ['quilt', 'fabric'],
  mohist: ['forge', 'spigot', 'bukkit'],
  arclight: ['forge', 'spigot', 'bukkit'],
  sponge: ['sponge'],
  unknown: [],
};

/** Whether a loader supports plugins, mods, or both */
export const LOADER_PLATFORM: Record<ServerLoader, ServerPlatform> = {
  vanilla: 'vanilla',
  paper: 'plugins',
  spigot: 'plugins',
  bukkit: 'plugins',
  purpur: 'plugins',
  pufferfish: 'plugins',
  folia: 'plugins',
  leaves: 'plugins',
  fabric: 'mods',
  forge: 'mods',
  neoforge: 'mods',
  quilt: 'mods',
  mohist: 'both',
  arclight: 'both',
  sponge: 'both',
  unknown: 'unknown',
};

/** Which content tabs to show for each platform */
export function getAvailableTabs(platform: ServerPlatform): Array<'plugins' | 'mods' | 'datapacks'> {
  switch (platform) {
    case 'vanilla': return ['datapacks'];
    case 'plugins': return ['plugins', 'datapacks'];
    case 'mods': return ['mods', 'datapacks'];
    case 'both': return ['plugins', 'mods', 'datapacks'];
    case 'unknown': return ['plugins', 'mods', 'datapacks'];
  }
}

// ─── File system helpers ─────────────────────────────────────

interface DirEntry {
  name: string;
  directory: boolean;
  file: boolean;
}

async function listDir(uuid: string, path: string): Promise<DirEntry[]> {
  try {
    const { data } = await axiosInstance.get(`/api/client/servers/${uuid}/files/list`, {
      params: { directory: path, page: 1, per_page: 100, sort: 'name_asc' },
    });
    return (data.entries?.data ?? []) as DirEntry[];
  } catch {
    return [];
  }
}

async function readFile(uuid: string, path: string): Promise<string | null> {
  try {
    const { data } = await axiosInstance.get(`/api/client/servers/${uuid}/files/contents`, {
      params: { file: path },
      responseType: 'text',
      transformResponse: [(d: string) => d],
    });
    return data;
  } catch {
    return null;
  }
}

async function fileExists(entries: DirEntry[], name: string, type: 'file' | 'dir'): Promise<boolean> {
  return entries.some((e) =>
    e.name === name && (type === 'file' ? e.file : e.directory),
  );
}

// ─── Detection helpers ───────────────────────────────────────

/** MCJars type identifiers mapped to our loader types */
const MCJARS_TO_LOADER: Record<string, ServerLoader> = {
  VANILLA: 'vanilla',
  PAPER: 'paper',
  SPIGOT: 'spigot',
  BUKKIT: 'bukkit',
  PURPUR: 'purpur',
  PUFFERFISH: 'pufferfish',
  FOLIA: 'folia',
  FABRIC: 'fabric',
  FORGE: 'forge',
  NEOFORGE: 'neoforge',
  QUILT: 'quilt',
  MOHIST: 'mohist',
  ARCLIGHT: 'arclight',
  SPONGE: 'sponge',
  LEAVES: 'leaves',
  CANVAS: 'paper',
};

/**
 * Read .mcvc-type.json written by the MC Version Chooser extension.
 * This is the highest priority signal.
 */
interface McvcMarker {
  loader: ServerLoader | null;
  version: string | null;
}

async function detectFromMcvcMarker(uuid: string): Promise<McvcMarker> {
  const content = await readFile(uuid, '/.mcvc-type.json');
  if (!content) return { loader: null, version: null };
  try {
    const data = JSON.parse(content);
    return {
      loader: (data.type && MCJARS_TO_LOADER[data.type]) ? MCJARS_TO_LOADER[data.type] : null,
      version: data.version ?? null,
    };
  } catch { /* ignore */ }
  return { loader: null, version: null };
}

/** Read level-name from server.properties to find the world directory */
async function getWorldDir(uuid: string): Promise<string> {
  const content = await readFile(uuid, '/server.properties');
  if (content) {
    const match = content.match(/^level-name\s*=\s*(.+)$/m);
    if (match && match[1].trim()) return match[1].trim();
  }
  return 'world';
}

/**
 * Detect server type using a deterministic decision tree.
 *
 * Priority 1: .mcvc-type.json marker (from Version Chooser extension)
 * Priority 2: Directory/file fingerprints (checks most specific first)
 *
 * The order matters because server types inherit from each other:
 *   Purpur > Pufferfish > Paper > Spigot > CraftBukkit
 *   NeoForge vs Forge (both use libraries/)
 *   Folia is a Paper fork
 *   Leaves is a Paper fork
 *
 * Decision tree (Bukkit-chain checked BEFORE libraries/ because Paper
 * bundles NeoForge libraries as dependencies, causing false positives):
 *   purpur.yml                     → PURPUR
 *   pufferfish.yml                 → PUFFERFISH
 *   leaves.yml                     → LEAVES
 *   config/folia-global.yml        → FOLIA
 *   config/paper-global.yml        → PAPER
 *   spigot.yml                     → SPIGOT
 *   bukkit.yml                     → CRAFTBUKKIT
 *   fabric-server-launch*.jar / .fabric/ → FABRIC
 *   quilt-server-launcher*.jar / .quilt/ → QUILT
 *   libraries/net/neoforged/       → NEOFORGE
 *   libraries/net/minecraftforge/  → FORGE
 *   server.properties              → VANILLA
 *   else                           → UNKNOWN
 */
export async function detectServer(uuid: string, eggName: string, startup: string, dockerImage: string): Promise<ServerDetection> {
  const result: ServerDetection = {
    platform: 'unknown',
    loader: 'unknown',
    mcVersion: null,
    hasPluginsDir: false,
    hasModsDir: false,
    worldDir: 'world',
    worldDirs: [],
  };

  // ── Priority 1: .mcvc-type.json marker ──
  const mcvcMarker = await detectFromMcvcMarker(uuid);
  if (mcvcMarker.loader) {
    result.loader = mcvcMarker.loader;
    result.platform = LOADER_PLATFORM[mcvcMarker.loader];
  }
  if (mcvcMarker.version) {
    result.mcVersion = mcvcMarker.version;
  }

  // ── List root directory ──
  const rootEntries = await listDir(uuid, '/');
  const rootDirs = new Set(rootEntries.filter((e) => e.directory).map((e) => e.name));
  const rootFiles = new Set(rootEntries.filter((e) => e.file).map((e) => e.name));

  result.hasPluginsDir = rootDirs.has('plugins');
  result.hasModsDir = rootDirs.has('mods');

  // ── Priority 2: File fingerprint decision tree ──
  // Bukkit-chain checked FIRST because Paper bundles NeoForge/Forge libraries
  // as dependencies, so libraries/net/neoforged/ alone is NOT reliable.
  if (result.loader === 'unknown') {

    // ── Bukkit-chain (most specific first) ──

    // Purpur — purpur.yml (includes Pufferfish + Paper + Spigot + Bukkit)
    if (rootFiles.has('purpur.yml')) {
      result.loader = 'purpur';
    }
    // Pufferfish — pufferfish.yml but no purpur.yml
    else if (rootFiles.has('pufferfish.yml')) {
      result.loader = 'pufferfish';
    }
    // Leaves — leaves.yml (Paper fork)
    else if (rootFiles.has('leaves.yml')) {
      result.loader = 'leaves';
    }
    // Folia / Paper — need to check config/ directory
    else if (rootDirs.has('config')) {
      const configEntries = await listDir(uuid, '/config');
      const configDirs = new Set(configEntries.filter((e) => e.directory).map((e) => e.name));
      const configFiles = new Set(configEntries.filter((e) => e.file).map((e) => e.name));

      if (configDirs.has('sponge')) {
        result.loader = 'sponge';
      } else if (configFiles.has('folia-global.yml')) {
        result.loader = 'folia';
      } else if (configFiles.has('paper-global.yml')) {
        result.loader = 'paper';
      }
    }
    // Older Paper (paper-global.yml in root instead of config/)
    if (result.loader === 'unknown' && rootFiles.has('paper-global.yml')) {
      result.loader = 'paper';
    }
    // Spigot — spigot.yml but no Paper config
    if (result.loader === 'unknown' && rootFiles.has('spigot.yml')) {
      result.loader = 'spigot';
    }
    // CraftBukkit — bukkit.yml but no spigot.yml
    if (result.loader === 'unknown' && rootFiles.has('bukkit.yml')) {
      result.loader = 'bukkit';
    }

    // ── Mod loaders (only if no Bukkit-chain match) ──

    // Fabric
    if (result.loader === 'unknown') {
      if (rootDirs.has('.fabric')
        || rootFiles.has('fabric-server-launch.jar')
        || rootFiles.has('fabric-server-launcher.jar')) {
        result.loader = 'fabric';
      }
    }
    // Quilt
    if (result.loader === 'unknown') {
      if (rootDirs.has('.quilt')
        || rootFiles.has('quilt-server-launch.jar')
        || rootFiles.has('quilt-server-launcher.jar')) {
        result.loader = 'quilt';
      }
    }
    // NeoForge / Forge — check libraries/ ONLY if nothing else matched
    if (result.loader === 'unknown' && rootDirs.has('libraries')) {
      const libNetEntries = await listDir(uuid, '/libraries/net');
      const libNetDirs = new Set(libNetEntries.filter((e) => e.directory).map((e) => e.name));

      if (libNetDirs.has('neoforged')) {
        result.loader = 'neoforge';
      } else if (libNetDirs.has('minecraftforge')) {
        result.loader = 'forge';
      }
    }

    // ── Vanilla — nothing else matched ──
    if (result.loader === 'unknown' && rootFiles.has('server.properties')) {
      result.loader = 'vanilla';
    }
  }

  // ── Set platform from loader ──
  if (result.loader !== 'unknown') {
    result.platform = LOADER_PLATFORM[result.loader];
  }
  // Extra check: both plugins/ and mods/ = hybrid even if loader is unknown
  if (result.hasPluginsDir && result.hasModsDir && result.platform !== 'both') {
    if (result.platform === 'plugins') result.platform = 'both';
    if (result.platform === 'mods') result.platform = 'both';
  }

  // ── Detect MC version (multiple strategies) ──
  // mcVersion may already be set from .mcvc-type.json marker — these override it
  // with more accurate sources if available.

  // Strategy 1: version.json in root (vanilla + Bukkit-chain generate this on boot)
  const versionJson = await readFile(uuid, '/version.json');
  if (versionJson) {
    try {
      const parsed = JSON.parse(versionJson);
      if (parsed.id) result.mcVersion = parsed.id;
      else if (parsed.name) result.mcVersion = parsed.name;
    } catch { /* ignore */ }
  }

  // Strategy 2: Forge — version in libraries/net/minecraftforge/forge/<MC>-<FORGE>/
  if (!result.mcVersion && (result.loader === 'forge' || result.loader === 'mohist' || result.loader === 'arclight')) {
    const forgeDirs = await listDir(uuid, '/libraries/net/minecraftforge/forge');
    if (forgeDirs.length > 0) {
      // Folder name format: "1.20.1-47.3.0" — MC version is before the dash
      const folderName = forgeDirs[0].name;
      const dashIdx = folderName.indexOf('-');
      if (dashIdx > 0) result.mcVersion = folderName.substring(0, dashIdx);
    }
  }

  // Strategy 3: NeoForge — version in libraries/net/neoforged/neoforge/<VERSION>/
  // NeoForge 21.x = MC 1.21.x, 20.x = MC 1.20.x, etc.
  if (!result.mcVersion && result.loader === 'neoforge') {
    const neoforgeDirs = await listDir(uuid, '/libraries/net/neoforged/neoforge');
    if (neoforgeDirs.length > 0) {
      const folderName = neoforgeDirs[0].name;
      // Format: "21.1.77" → MC 1.21.1, "20.4.237" → MC 1.20.4
      const parts = folderName.split('.');
      if (parts.length >= 2) {
        result.mcVersion = `1.${parts[0]}.${parts[1]}`;
      }
    }
  }

  // Strategy 4: logs/latest.log — universal fallback, every server prints the version
  if (!result.mcVersion) {
    const log = await readFile(uuid, '/logs/latest.log');
    if (log) {
      const match = log.substring(0, 3000).match(/Starting minecraft server version\s+([^\s\n]+)/i);
      if (match) result.mcVersion = match[1];
    }
  }

  // ── Get world directory name for datapacks ──
  result.worldDir = await getWorldDir(uuid);

  // ── Discover all world directories (folders containing level.dat) ──
  const worldDirs: string[] = [];
  for (const entry of rootEntries) {
    if (entry.directory) {
      const worldFiles = await listDir(uuid, `/${entry.name}`);
      if (worldFiles.some((f) => f.file && f.name === 'level.dat')) {
        worldDirs.push(entry.name);
      }
    }
  }
  result.worldDirs = worldDirs.length > 0 ? worldDirs : [result.worldDir];

  return result;
}
